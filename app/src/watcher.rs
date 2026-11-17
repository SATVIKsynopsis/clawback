//! Probes monitors, detects breaches, and drives each incident through
//! proof (Telegraph) -> claim.

use crate::claim;
use crate::config::{Config, MonitorCfg};
use crate::db::{self, Db, Probe};
use crate::proof::{self, Budget};
use crate::sla;
use crate::telegraph::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Ctx {
    pub cfg: Config,
    pub db: Arc<Db>,
    pub tg: Arc<Client>,
    pub http: reqwest::Client,
}

pub fn target_for(m: &MonitorCfg) -> f64 {
    m.breach_below_pct.or_else(|| sla::policy_for(&m.provider).map(|p| p.target_pct)).unwrap_or(99.9)
}

pub async fn probe(http: &reqwest::Client, url: &str) -> Probe {
    let t0 = Instant::now();
    let res = http.get(url).send().await;
    let latency_ms = t0.elapsed().as_millis() as u64;
    match res {
        Ok(r) => {
            let s = r.status();
            Probe { ts: db::now(), ok: s.is_success(), status: Some(s.as_u16()), latency_ms, error: None }
        }
        Err(e) => Probe { ts: db::now(), ok: false, status: None, latency_ms, error: Some(e.to_string()) },
    }
}

pub fn observed_uptime(probes: &[Probe]) -> Option<f64> {
    if probes.is_empty() {
        return None;
    }
    let ok = probes.iter().filter(|p| p.ok).count() as f64;
    Some((ok / probes.len() as f64 * 10_000.0).round() / 100.0)
}

/// Contiguous runs of failed probes: (first failure, first success after it | last failure + interval).
pub fn outage_windows(probes: &[Probe], interval_secs: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut start: Option<i64> = None;
    let mut last_fail = 0;
    for p in probes {
        match (p.ok, start) {
            (false, None) => {
                start = Some(p.ts);
                last_fail = p.ts;
            }
            (false, Some(_)) => last_fail = p.ts,
            (true, Some(s)) => {
                out.push((s, p.ts));
                start = None;
            }
            (true, None) => {}
        }
    }
    if let Some(s) = start {
        out.push((s, last_fail + interval_secs));
    }
    out
}

/// After each probe: open an incident on breach, track/close an active one on recovery.
/// Returns a newly opened incident id.
pub fn evaluate(ctx: &Ctx, m: &MonitorCfg) -> Option<i64> {
    let probes = ctx.db.probes_since(&m.name, db::now() - m.window_secs as i64);
    if probes.len() < m.min_samples {
        return None;
    }
    let up = observed_uptime(&probes)?;
    let target = target_for(m);
    match ctx.db.active_incident(&m.name) {
        Some(inc) => {
            ctx.db.set_observed(inc.id, up);
            if up >= target && inc.status != "proving" {
                ctx.db.close_incident(inc.id);
                ctx.db.event(Some(inc.id), "info", &format!("{}: recovered, observed uptime {up}% over {}s", m.name, m.window_secs));
            }
            None
        }
        None if up < target => {
            let id = ctx.db.open_incident(&m.name, &m.service(), &m.period, up);
            ctx.db.event(
                Some(id),
                "warn",
                &format!("{}: observed uptime {up}% over last {}s is below SLA {target}%; incident opened", m.name, m.window_secs),
            );
            Some(id)
        }
        None => None,
    }
}

pub async fn handle_incident(ctx: Arc<Ctx>, id: i64, m: MonitorCfg) {
    let db = &ctx.db;
    db.set_status(id, "proving", "buying independent SLA proof via Telegraph");
    let budget = Budget { db, incident_id: Some(id), per_incident: ctx.cfg.budget.per_incident_uusdc, per_day: ctx.cfg.budget.per_day_uusdc };
    let service = m.service();
    let proof =
        match proof::obtain(&ctx.tg, db, &budget, Some(id), &service, &m.period, ctx.cfg.telegraph.fallback_miner.as_deref()).await {
            Ok(p) => p,
            Err(f) => {
                db.set_status(id, "unproven", &format!("no validated proof, no claim filed: {}", f.reasons.join(" | ")));
                return;
            }
        };
    db.set_proof(id, proof.uptime_pct, proof.sla_target_pct);
    if proof.compliant {
        db.set_status(
            id,
            "not_confirmed",
            &format!(
                "independent proof says {} met its SLA ({}% >= {}%) over {}; our outage was not provider-wide, no claim",
                proof.service, proof.uptime_pct, proof.sla_target_pct, proof.period
            ),
        );
        return;
    }
    let Some(inc) = db.incident(id) else { return };
    let probes = db.probes_since(&m.name, inc.opened_at - m.window_secs as i64);
    let windows = outage_windows(&probes, ctx.cfg.watcher.interval_secs as i64);
    let purchases = db.purchases(Some(id));
    match claim::draft(&ctx.cfg, &inc, &m, &proof, &windows, &purchases).await {
        Ok(d) => {
            db.set_claim(id, d.credit.as_ref().map_or(0.0, |c| c.credit_usd), &d.path);
            let status = if d.credit.is_some() { "claimed" } else { "no_credit" };
            db.set_status(id, status, &d.summary);
            db.event(Some(id), "info", &d.summary);
        }
        Err(e) => db.set_status(id, "claim_failed", &e),
    }
}

pub async fn run_monitor(ctx: Arc<Ctx>, m: MonitorCfg) {
    let mut tick = tokio::time::interval(Duration::from_secs(ctx.cfg.watcher.interval_secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let p = probe(&ctx.http, &m.url).await;
        tracing::debug!(monitor = %m.name, ok = p.ok, status = p.status, latency_ms = p.latency_ms, "probe");
        ctx.db.insert_probe(&m.name, &p);
        if let Some(id) = evaluate(&ctx, &m) {
            tokio::spawn(handle_incident(ctx.clone(), id, m.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_config;
    use crate::telegraph::fake::*;

    fn p(ts: i64, ok: bool) -> Probe {
        Probe { ts, ok, status: Some(if ok { 200 } else { 503 }), latency_ms: 1, error: None }
    }

    #[test]
    fn uptime_and_windows() {
        let ps = vec![p(0, true), p(10, false), p(20, false), p(30, true), p(40, false)];
        assert_eq!(observed_uptime(&ps), Some(40.0));
        assert_eq!(outage_windows(&ps, 10), vec![(10, 30), (40, 50)]);
        assert_eq!(observed_uptime(&[]), None);
    }

    async fn ctx_with(node: &str, cfg_mut: impl FnOnce(&mut Config)) -> Arc<Ctx> {
        let mut cfg = test_config();
        cfg.telegraph.node = node.into();
        cfg.claim.out_dir = std::env::temp_dir().join(format!("clawback-test-{}", uuid::Uuid::new_v4())).to_string_lossy().into();
        cfg_mut(&mut cfg);
        Arc::new(Ctx { db: Arc::new(Db::memory()), tg: Arc::new(client(node)), http: reqwest::Client::new(), cfg })
    }

    fn seed_failures(ctx: &Ctx, m: &MonitorCfg) {
        let now = db::now();
        for i in 0..6 {
            ctx.db.insert_probe(&m.name, &p(now - 50 + i * 10, i < 2));
        }
    }

    #[tokio::test]
    async fn breach_to_validated_proof_to_claim() {
        let (node, _) = spawn(FakeState {
            routed: Some(sla_answer("Cloudflare", 100.0, 100.0)),
            direct: Some(sla_answer("GitHub", 99.2053, 99.9)),
            price: 10_000,
            ..Default::default()
        })
        .await;
        let ctx = ctx_with(&node, |_| {}).await;
        let m = ctx.cfg.monitors[0].clone();
        seed_failures(&ctx, &m);
        let id = evaluate(&ctx, &m).expect("incident opened");
        assert!(evaluate(&ctx, &m).is_none(), "no duplicate incident");
        handle_incident(ctx.clone(), id, m).await;
        let inc = ctx.db.incident(id).unwrap();
        assert_eq!(inc.status, "claimed", "{}", inc.detail);
        assert_eq!(inc.credit_usd, Some(210.0)); // 10% of $2100 (GitHub tier 99.0-99.5)
        let eml = std::fs::read_to_string(inc.claim_path.unwrap()).unwrap();
        assert!(eml.contains("0xreceipt2") && eml.contains("0xreceipt1"), "both receipts cited (rejected one marked)");
        assert!(eml.contains("telegraph-evidence.json"));
    }

    #[tokio::test]
    async fn compliant_provider_means_no_claim() {
        let (node, _) = spawn(FakeState { routed: Some(sla_answer("GitHub", 99.95, 99.9)), price: 10_000, ..Default::default() }).await;
        let ctx = ctx_with(&node, |_| {}).await;
        let m = ctx.cfg.monitors[0].clone();
        seed_failures(&ctx, &m);
        let id = evaluate(&ctx, &m).unwrap();
        handle_incident(ctx.clone(), id, m).await;
        let inc = ctx.db.incident(id).unwrap();
        assert_eq!(inc.status, "not_confirmed");
        assert!(inc.claim_path.is_none());
    }

    #[tokio::test]
    async fn zero_budget_incident_is_unproven_with_reason() {
        let (node, _) = spawn(FakeState { routed: Some(sla_answer("GitHub", 99.2, 99.9)), price: 10_000, ..Default::default() }).await;
        let ctx = ctx_with(&node, |c| c.budget.per_incident_uusdc = 0).await;
        let m = ctx.cfg.monitors[0].clone();
        seed_failures(&ctx, &m);
        let id = evaluate(&ctx, &m).unwrap();
        handle_incident(ctx.clone(), id, m).await;
        let inc = ctx.db.incident(id).unwrap();
        assert_eq!(inc.status, "unproven");
        assert!(inc.detail.contains("budget is 0"), "{}", inc.detail);
        assert!(ctx.db.purchases(Some(id)).is_empty());
    }

    #[tokio::test]
    async fn telegraph_down_means_no_claim() {
        let ctx = ctx_with("http://127.0.0.1:9", |_| {}).await;
        let m = ctx.cfg.monitors[0].clone();
        seed_failures(&ctx, &m);
        let id = evaluate(&ctx, &m).unwrap();
        handle_incident(ctx.clone(), id, m).await;
        let inc = ctx.db.incident(id).unwrap();
        assert_eq!(inc.status, "unproven");
        assert!(inc.detail.contains("unreachable"));
        assert!(inc.claim_path.is_none());
    }
}
