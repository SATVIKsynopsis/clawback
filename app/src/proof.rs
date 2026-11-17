//! Buy SLA_COMPLIANCE_GENERAL through Telegraph and validate the answer before trusting it.
//! Routed ask first; if the answer fails validation (or no routed miner can serve), record it as a
//! rejected purchase (its cost and receipt are kept) and retry once with a direct ask to the
//! configured fallback miner. A claim is never filed without a validated proof.

use crate::db::{self, Db, Purchase};
use crate::telegraph::{AskError, Client, Paid};
use serde::Serialize;
use serde_json::{json, Value};

pub const INTENT: &str = "SLA_COMPLIANCE_GENERAL";

#[derive(Debug, Clone, Serialize)]
pub struct SlaProof {
    pub service: String,
    pub uptime_pct: f64,
    pub sla_target_pct: f64,
    pub compliant: bool,
    pub period: String,
    pub miner_id: String,
    pub miner_name: String,
    pub receipt_hash: String,
    pub signal_hash: String,
    pub amount_uusdc: u64,
    pub mode: String,
    pub purchase_id: i64,
    /// Present when the miner returns evidence (our sla-miner does).
    pub outages: Option<Value>,
    pub sla_source: Option<String>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn status_ok(v: &Value) -> bool {
    match v {
        Value::String(s) => s.eq_ignore_ascii_case("ok"),
        Value::Object(o) => o.get("status").is_some_and(status_ok),
        _ => false,
    }
}

/// Returns the fields we rely on, or every reason the answer is unusable.
pub fn validate(asked_service: &str, body: &Value) -> Result<(String, f64, f64, bool, String), Vec<String>> {
    let mut why = Vec::new();
    let shape = body.get("result_shape").and_then(Value::as_str).unwrap_or("missing");
    if shape != "intent" {
        let extra = body.get("shape_error").and_then(Value::as_str).map(|e| format!(" ({e})")).unwrap_or_default();
        why.push(format!("result_shape is '{shape}'{extra}, expected 'intent'"));
    }
    let r = body.get("result").cloned().unwrap_or(Value::Null);
    let service = r.get("service").and_then(Value::as_str).unwrap_or_default().to_string();
    if service.is_empty() {
        why.push("result.service missing".into());
    } else if !norm(&service).contains(&norm(asked_service)) {
        why.push(format!("answered for '{service}', but we asked about '{asked_service}'"));
    }
    let num = |k: &str| r.get(k).and_then(Value::as_f64);
    let uptime = num("uptime_pct");
    match uptime {
        None => why.push("result.uptime_pct missing or not a number".into()),
        Some(u) if !(0.0..=100.0).contains(&u) => why.push(format!("uptime_pct {u} outside 0..100")),
        _ => {}
    }
    let target = num("sla_target_pct");
    match target {
        None => why.push("result.sla_target_pct missing or not a number".into()),
        Some(t) if !(t > 0.0 && t <= 100.0) => why.push(format!("sla_target_pct {t} outside (0,100]")),
        _ => {}
    }
    let compliant = r.get("compliant").and_then(Value::as_bool);
    if compliant.is_none() {
        why.push("result.compliant missing or not a bool".into());
    }
    if let (Some(u), Some(t), Some(c)) = (uptime, target, compliant)
        && c != (u >= t - 1e-6)
    {
        why.push(format!("inconsistent: compliant={c} but uptime {u} vs target {t}"));
    }
    let period = r.get("period").and_then(Value::as_str).unwrap_or_default().to_string();
    match body.get("field_status") {
        Some(Value::Object(fs)) => {
            let bad: Vec<String> = fs.iter().filter(|(_, v)| !status_ok(v)).map(|(k, v)| format!("{k}={v}")).collect();
            if !bad.is_empty() {
                why.push(format!("field_status not ok: {}", bad.join(", ")));
            }
        }
        Some(Value::Array(a)) => {
            let bad: Vec<String> = a.iter().filter(|v| !status_ok(v)).map(|v| v.to_string()).collect();
            if !bad.is_empty() {
                why.push(format!("field_status not ok: {}", bad.join(", ")));
            }
        }
        _ => why.push("field_status missing".into()),
    }
    if why.is_empty() {
        Ok((service, uptime.unwrap(), target.unwrap(), compliant.unwrap(), period))
    } else {
        Err(why)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProofFailure {
    pub reasons: Vec<String>,
}

pub struct Budget<'a> {
    pub db: &'a Db,
    pub incident_id: Option<i64>,
    pub per_incident: u64,
    pub per_day: u64,
}

impl Budget<'_> {
    pub fn check(&self, price: u64) -> Result<(), String> {
        if self.per_incident == 0 || self.per_day == 0 {
            return Err("budget is 0 (per_incident_uusdc or per_day_uusdc); no paid proof allowed".into());
        }
        if let Some(id) = self.incident_id {
            let spent = self.db.spent_for_incident(id);
            if spent + price > self.per_incident {
                return Err(format!("incident budget: spent {spent} + {price} > {} uUSDC", self.per_incident));
            }
        }
        let day = self.db.spent_today();
        if day + price > self.per_day {
            return Err(format!("daily budget: spent {day} + {price} > {} uUSDC", self.per_day));
        }
        Ok(())
    }
}

fn record(db: &Db, incident_id: Option<i64>, service: &str, mode: &str, paid: &Paid, verdict: &Result<(String, f64, f64, bool, String), Vec<String>>) -> i64 {
    db.insert_purchase(&Purchase {
        incident_id,
        ts: db::now(),
        mode: mode.into(),
        intent: INTENT.into(),
        service: service.into(),
        miner_id: paid.miner_id(),
        miner_name: paid.miner_name(),
        amount_uusdc: paid.amount_uusdc,
        receipt_hash: paid.receipt_hash(),
        signal_hash: paid.signal_hash(),
        valid: verdict.is_ok(),
        reject_reasons: verdict.as_ref().err().cloned().unwrap_or_default(),
        response: paid.body.clone(),
        receipt: paid.receipt.clone(),
        ..Default::default()
    })
}

/// Routed ask, validated; on rejection or a retryable error, one direct ask to `fallback_miner`.
pub async fn obtain(
    tg: &Client,
    db: &Db,
    budget: &Budget<'_>,
    incident_id: Option<i64>,
    service: &str,
    period: &str,
    fallback_miner: Option<&str>,
) -> Result<SlaProof, ProofFailure> {
    let params = json!({ "service": service, "period": period });
    let mut reasons = Vec::new();
    let attempts: Vec<(&str, Option<&str>)> = std::iter::once(("routed", None)).chain(fallback_miner.map(|m| ("direct", Some(m)))).collect();

    for (mode, miner) in attempts {
        let res = match miner {
            None => tg.ask(INTENT, params.clone(), |p| budget.check(p)).await,
            Some(m) => tg.ask_direct(m, INTENT, params.clone(), |p| budget.check(p)).await,
        };
        let paid = match res {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("{mode} ask failed: {e}");
                db.event(incident_id, if matches!(e, AskError::BudgetDenied { .. }) { "warn" } else { "error" }, &msg);
                reasons.push(msg);
                if e.retryable() {
                    continue;
                }
                return Err(ProofFailure { reasons });
            }
        };
        let verdict = validate(service, &paid.body);
        let purchase_id = record(db, incident_id, service, mode, &paid, &verdict);
        match verdict {
            Ok((svc, uptime, target, compliant, per)) => {
                db.event(
                    incident_id,
                    "info",
                    &format!(
                        "validated proof from {} ({}): {svc} uptime {uptime}% vs target {target}% over {per}, receipt {}",
                        paid.miner_name(),
                        mode,
                        paid.receipt_hash()
                    ),
                );
                let r = &paid.body["result"];
                return Ok(SlaProof {
                    service: svc,
                    uptime_pct: uptime,
                    sla_target_pct: target,
                    compliant,
                    period: per,
                    miner_id: paid.miner_id(),
                    miner_name: paid.miner_name(),
                    receipt_hash: paid.receipt_hash(),
                    signal_hash: paid.signal_hash(),
                    amount_uusdc: paid.amount_uusdc,
                    mode: mode.into(),
                    purchase_id,
                    outages: r.get("outages").cloned().filter(|v| !v.is_null()),
                    sla_source: r.get("sla_source").and_then(Value::as_str).map(str::to_string),
                });
            }
            Err(why) => {
                let msg = format!("rejected {mode} answer from {} (paid {} uUSDC): {}", paid.miner_name(), paid.amount_uusdc, why.join("; "));
                db.event(incident_id, "warn", &msg);
                reasons.push(msg);
            }
        }
    }
    if fallback_miner.is_none() {
        reasons.push("no fallback_miner configured for a direct-ask retry".into());
    }
    Err(ProofFailure { reasons })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegraph::fake::*;

    #[test]
    fn validates_good_answer() {
        let (svc, u, t, c, _) = validate("GitHub", &sla_answer("GitHub", 99.2053, 99.9)).unwrap();
        assert_eq!((svc.as_str(), u, t, c), ("GitHub", 99.2053, 99.9, false));
        assert!(validate("GitHub", &sla_answer("GitHub Actions", 99.6, 99.9)).is_ok());
    }

    #[test]
    fn rejects_the_real_before_answer() {
        // Shape of the Oct 31 routed answer: Cloudflare current status for a GitHub question.
        let body = json!({ "miner_name": "Cloudflare Status SLA", "result_shape": "raw", "shape_error": "no_fields_found",
            "result": { "page": {"name": "Cloudflare Status"}, "status": {"indicator": "major"} } });
        let why = validate("GitHub", &body).unwrap_err();
        assert!(why[0].contains("raw") && why[0].contains("no_fields_found"));
        assert!(why.iter().any(|w| w.contains("service missing")));
    }

    #[test]
    fn rejects_real_routed_answer_2026_11_17() {
        // evidence/comparison/routed.response.json: top-ranked miner, charged $0.01, no numbers.
        let body: Value = serde_json::from_str(include_str!("../../evidence/comparison/routed.response.json")).unwrap();
        let why = validate("GitHub", &body).unwrap_err();
        assert!(why.iter().any(|w| w.contains("uptime_pct missing")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("field_status not ok") && w.contains("rejected")), "{why:?}");
    }

    #[test]
    fn rejects_wrong_service_range_and_field_status() {
        assert!(validate("GitHub", &sla_answer("Cloudflare", 100.0, 100.0)).unwrap_err()[0].contains("Cloudflare"));
        let mut b = sla_answer("GitHub", 120.0, 99.9);
        b["result"]["compliant"] = json!(true);
        assert!(validate("GitHub", &b).unwrap_err().iter().any(|w| w.contains("outside 0..100")));
        let mut b = sla_answer("GitHub", 99.0, 99.9);
        b["field_status"]["uptime_pct"] = json!("missing");
        assert!(validate("GitHub", &b).unwrap_err()[0].contains("field_status"));
        let mut b = sla_answer("GitHub", 99.0, 99.9);
        b["result"]["compliant"] = json!(true);
        assert!(validate("GitHub", &b).unwrap_err()[0].contains("inconsistent"));
    }

    fn budget(db: &Db, id: Option<i64>, per_incident: u64) -> Budget<'_> {
        Budget { db, incident_id: id, per_incident, per_day: 100_000 }
    }

    #[tokio::test]
    async fn wrong_service_is_rejected_then_direct_retry_succeeds() {
        let (node, st) = spawn(FakeState {
            routed: Some(sla_answer("Cloudflare", 100.0, 100.0)),
            direct: Some(sla_answer("GitHub", 99.2053, 99.9)),
            price: 10_000,
            ..Default::default()
        })
        .await;
        let db = Db::memory();
        let id = db.open_incident("demo", "GitHub", "last 30 days", 90.0);
        let proof = obtain(&client(&node), &db, &budget(&db, Some(id), 30_000), Some(id), "GitHub", "last 30 days", Some("7410311"))
            .await
            .unwrap();
        assert_eq!((proof.mode.as_str(), proof.uptime_pct, proof.compliant), ("direct", 99.2053, false));
        let ps = db.purchases(Some(id));
        assert_eq!(ps.len(), 2);
        assert!(!ps[0].valid && ps[0].reject_reasons[0].contains("Cloudflare"));
        assert!(ps[1].valid);
        assert_eq!(db.spent_for_incident(id), 20_000); // rejected answer still cost money and is recorded
        assert_eq!(st.lock().unwrap().paid_calls.len(), 2);
    }

    #[tokio::test]
    async fn incident_budget_stops_the_retry() {
        let (node, _) = spawn(FakeState {
            routed: Some(sla_answer("Cloudflare", 100.0, 100.0)),
            direct: Some(sla_answer("GitHub", 99.2, 99.9)),
            price: 10_000,
            ..Default::default()
        })
        .await;
        let db = Db::memory();
        let id = db.open_incident("demo", "GitHub", "last 30 days", 90.0);
        let f = obtain(&client(&node), &db, &budget(&db, Some(id), 15_000), Some(id), "GitHub", "last 30 days", Some("7410311"))
            .await
            .unwrap_err();
        assert!(f.reasons.last().unwrap().contains("incident budget"), "{:?}", f.reasons);
        assert_eq!(db.purchases(Some(id)).len(), 1);
    }

    #[tokio::test]
    async fn zero_budget_never_pays() {
        let (node, st) = spawn(FakeState { routed: Some(sla_answer("GitHub", 99.2, 99.9)), price: 10_000, ..Default::default() }).await;
        let db = Db::memory();
        let f = obtain(&client(&node), &db, &budget(&db, None, 0), None, "GitHub", "last 30 days", Some("7410311")).await.unwrap_err();
        assert!(f.reasons[0].contains("budget is 0"));
        assert!(st.lock().unwrap().paid_calls.is_empty());
        assert!(db.purchases(None).is_empty());
    }

    #[tokio::test]
    async fn unreachable_node_gives_clear_error_and_no_retry() {
        let db = Db::memory();
        let f = obtain(&client("http://127.0.0.1:9"), &db, &budget(&db, None, 30_000), None, "GitHub", "last 30 days", Some("7410311"))
            .await
            .unwrap_err();
        assert_eq!(f.reasons.len(), 1);
        assert!(f.reasons[0].contains("unreachable"));
    }

    #[tokio::test]
    async fn no_routed_miner_falls_back_to_direct() {
        let (node, _) = spawn(FakeState { routed: None, direct: Some(sla_answer("GitHub", 99.95, 99.9)), price: 10_000, ..Default::default() }).await;
        let db = Db::memory();
        let p = obtain(&client(&node), &db, &budget(&db, None, 30_000), None, "GitHub", "last 30 days", Some("7410311")).await.unwrap();
        assert!(p.compliant);
        assert_eq!(p.mode, "direct");
    }
}
