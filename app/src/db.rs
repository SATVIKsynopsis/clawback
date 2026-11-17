//! SQLite store: probes, incidents, purchases (the expense ledger), events.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub ts: i64,
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Incident {
    pub id: i64,
    pub monitor: String,
    pub service: String,
    pub period: String,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    pub observed_uptime_pct: f64,
    /// open -> proving -> claimed | not_confirmed | unproven
    pub status: String,
    pub detail: String,
    pub independent_uptime_pct: Option<f64>,
    pub sla_target_pct: Option<f64>,
    pub credit_usd: Option<f64>,
    pub claim_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Purchase {
    pub id: i64,
    pub incident_id: Option<i64>,
    pub ts: i64,
    /// routed | direct
    pub mode: String,
    pub intent: String,
    pub service: String,
    pub miner_id: String,
    pub miner_name: String,
    pub amount_uusdc: u64,
    pub receipt_hash: String,
    pub signal_hash: String,
    pub valid: bool,
    pub reject_reasons: Vec<String>,
    pub response: serde_json::Value,
    pub receipt: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub ts: i64,
    pub incident_id: Option<i64>,
    pub level: String,
    pub message: String,
}

pub fn now() -> i64 {
    Utc::now().timestamp()
}

pub fn iso(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0).single().map(|t: DateTime<Utc>| t.to_rfc3339()).unwrap_or_default()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS probes (
  id INTEGER PRIMARY KEY, monitor TEXT NOT NULL, ts INTEGER NOT NULL, ok INTEGER NOT NULL,
  status INTEGER, latency_ms INTEGER NOT NULL, error TEXT);
CREATE INDEX IF NOT EXISTS probes_monitor_ts ON probes(monitor, ts);
CREATE TABLE IF NOT EXISTS incidents (
  id INTEGER PRIMARY KEY, monitor TEXT NOT NULL, service TEXT NOT NULL, period TEXT NOT NULL,
  opened_at INTEGER NOT NULL, closed_at INTEGER, observed_uptime_pct REAL NOT NULL,
  status TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '',
  independent_uptime_pct REAL, sla_target_pct REAL, credit_usd REAL, claim_path TEXT);
CREATE TABLE IF NOT EXISTS purchases (
  id INTEGER PRIMARY KEY, incident_id INTEGER, ts INTEGER NOT NULL, mode TEXT NOT NULL,
  intent TEXT NOT NULL, service TEXT NOT NULL, miner_id TEXT NOT NULL, miner_name TEXT NOT NULL,
  amount_uusdc INTEGER NOT NULL, receipt_hash TEXT NOT NULL, signal_hash TEXT NOT NULL,
  valid INTEGER NOT NULL, reject_reasons TEXT NOT NULL, response TEXT NOT NULL, receipt TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, incident_id INTEGER, level TEXT NOT NULL, message TEXT NOT NULL);
";

fn incident_row(r: &rusqlite::Row) -> rusqlite::Result<Incident> {
    Ok(Incident {
        id: r.get(0)?,
        monitor: r.get(1)?,
        service: r.get(2)?,
        period: r.get(3)?,
        opened_at: r.get(4)?,
        closed_at: r.get(5)?,
        observed_uptime_pct: r.get(6)?,
        status: r.get(7)?,
        detail: r.get(8)?,
        independent_uptime_pct: r.get(9)?,
        sla_target_pct: r.get(10)?,
        credit_usd: r.get(11)?,
        claim_path: r.get(12)?,
    })
}
const INCIDENT_COLS: &str = "id, monitor, service, period, opened_at, closed_at, observed_uptime_pct, status, detail, \
                             independent_uptime_pct, sla_target_pct, credit_usd, claim_path";

fn purchase_row(r: &rusqlite::Row) -> rusqlite::Result<Purchase> {
    let reasons: String = r.get(11)?;
    let response: String = r.get(12)?;
    let receipt: String = r.get(13)?;
    Ok(Purchase {
        id: r.get(0)?,
        incident_id: r.get(1)?,
        ts: r.get(2)?,
        mode: r.get(3)?,
        intent: r.get(4)?,
        service: r.get(5)?,
        miner_id: r.get(6)?,
        miner_name: r.get(7)?,
        amount_uusdc: r.get::<_, i64>(8)? as u64,
        receipt_hash: r.get(9)?,
        signal_hash: r.get(10)?,
        valid: r.get(14)?,
        reject_reasons: serde_json::from_str(&reasons).unwrap_or_default(),
        response: serde_json::from_str(&response).unwrap_or_default(),
        receipt: serde_json::from_str(&receipt).unwrap_or_default(),
    })
}
const PURCHASE_COLS: &str = "id, incident_id, ts, mode, intent, service, miner_id, miner_name, amount_uusdc, \
                             receipt_hash, signal_hash, reject_reasons, response, receipt, valid";

impl Db {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        Self { conn: Mutex::new(conn) }
    }

    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert_probe(&self, monitor: &str, p: &Probe) {
        let _ = self.c().execute(
            "INSERT INTO probes (monitor, ts, ok, status, latency_ms, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![monitor, p.ts, p.ok, p.status, p.latency_ms as i64, p.error],
        );
    }

    pub fn probes_since(&self, monitor: &str, since: i64) -> Vec<Probe> {
        let c = self.c();
        let mut st = c
            .prepare("SELECT ts, ok, status, latency_ms, error FROM probes WHERE monitor = ?1 AND ts >= ?2 ORDER BY ts")
            .unwrap();
        st.query_map(params![monitor, since], |r| {
            Ok(Probe { ts: r.get(0)?, ok: r.get(1)?, status: r.get(2)?, latency_ms: r.get::<_, i64>(3)? as u64, error: r.get(4)? })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn open_incident(&self, monitor: &str, service: &str, period: &str, observed: f64) -> i64 {
        let c = self.c();
        c.execute(
            "INSERT INTO incidents (monitor, service, period, opened_at, observed_uptime_pct, status) VALUES (?1, ?2, ?3, ?4, ?5, 'open')",
            params![monitor, service, period, now(), observed],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    pub fn active_incident(&self, monitor: &str) -> Option<Incident> {
        self.c()
            .query_row(
                &format!("SELECT {INCIDENT_COLS} FROM incidents WHERE monitor = ?1 AND closed_at IS NULL ORDER BY id DESC LIMIT 1"),
                params![monitor],
                incident_row,
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn incident(&self, id: i64) -> Option<Incident> {
        self.c()
            .query_row(&format!("SELECT {INCIDENT_COLS} FROM incidents WHERE id = ?1"), params![id], incident_row)
            .optional()
            .ok()
            .flatten()
    }

    pub fn incidents(&self, limit: usize) -> Vec<Incident> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {INCIDENT_COLS} FROM incidents ORDER BY id DESC LIMIT ?1")).unwrap();
        st.query_map(params![limit as i64], incident_row).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn set_status(&self, id: i64, status: &str, detail: &str) {
        let _ = self.c().execute("UPDATE incidents SET status = ?2, detail = ?3 WHERE id = ?1", params![id, status, detail]);
    }

    pub fn set_observed(&self, id: i64, observed: f64) {
        let _ = self.c().execute(
            "UPDATE incidents SET observed_uptime_pct = MIN(observed_uptime_pct, ?2) WHERE id = ?1",
            params![id, observed],
        );
    }

    pub fn set_proof(&self, id: i64, independent: f64, target: f64) {
        let _ = self.c().execute(
            "UPDATE incidents SET independent_uptime_pct = ?2, sla_target_pct = ?3 WHERE id = ?1",
            params![id, independent, target],
        );
    }

    pub fn set_claim(&self, id: i64, credit_usd: f64, path: &str) {
        let _ = self.c().execute("UPDATE incidents SET credit_usd = ?2, claim_path = ?3 WHERE id = ?1", params![id, credit_usd, path]);
    }

    pub fn close_incident(&self, id: i64) {
        let _ = self.c().execute("UPDATE incidents SET closed_at = ?2 WHERE id = ?1 AND closed_at IS NULL", params![id, now()]);
    }

    pub fn insert_purchase(&self, p: &Purchase) -> i64 {
        let c = self.c();
        c.execute(
            "INSERT INTO purchases (incident_id, ts, mode, intent, service, miner_id, miner_name, amount_uusdc, receipt_hash, \
             signal_hash, valid, reject_reasons, response, receipt) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                p.incident_id,
                p.ts,
                p.mode,
                p.intent,
                p.service,
                p.miner_id,
                p.miner_name,
                p.amount_uusdc as i64,
                p.receipt_hash,
                p.signal_hash,
                p.valid,
                serde_json::to_string(&p.reject_reasons).unwrap(),
                p.response.to_string(),
                p.receipt.to_string(),
            ],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    pub fn purchases(&self, incident_id: Option<i64>) -> Vec<Purchase> {
        let c = self.c();
        let (sql, arg) = match incident_id {
            Some(id) => (format!("SELECT {PURCHASE_COLS} FROM purchases WHERE incident_id = ?1 ORDER BY id"), id),
            None => (format!("SELECT {PURCHASE_COLS} FROM purchases WHERE ?1 = ?1 ORDER BY id DESC LIMIT 200"), 0),
        };
        let mut st = c.prepare(&sql).unwrap();
        st.query_map(params![arg], purchase_row).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn spent_for_incident(&self, id: i64) -> u64 {
        self.c()
            .query_row("SELECT COALESCE(SUM(amount_uusdc), 0) FROM purchases WHERE incident_id = ?1", params![id], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    pub fn spent_since(&self, since: i64) -> u64 {
        self.c()
            .query_row("SELECT COALESCE(SUM(amount_uusdc), 0) FROM purchases WHERE ts >= ?1", params![since], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    pub fn spent_today(&self) -> u64 {
        let midnight = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        self.spent_since(midnight)
    }

    pub fn event(&self, incident_id: Option<i64>, level: &str, message: &str) {
        match level {
            "error" => tracing::error!(incident_id, "{message}"),
            "warn" => tracing::warn!(incident_id, "{message}"),
            _ => tracing::info!(incident_id, "{message}"),
        }
        let _ = self.c().execute(
            "INSERT INTO events (ts, incident_id, level, message) VALUES (?1, ?2, ?3, ?4)",
            params![now(), incident_id, level, message],
        );
    }

    pub fn events(&self, limit: usize) -> Vec<Event> {
        let c = self.c();
        let mut st = c.prepare("SELECT ts, incident_id, level, message FROM events ORDER BY id DESC LIMIT ?1").unwrap();
        st.query_map(params![limit as i64], |r| Ok(Event { ts: r.get(0)?, incident_id: r.get(1)?, level: r.get(2)?, message: r.get(3)? }))
            .map(|r| r.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }
}
