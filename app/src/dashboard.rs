//! Dashboard: one static page + JSON state. Headline: credit claimed vs intelligence spent.

use crate::db;
use crate::watcher::{observed_uptime, target_for, Ctx};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

const INDEX: &str = include_str!("../static/index.html");

pub fn router(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(INDEX) }))
        .route("/api/state", get(state))
        .route("/claims/{id}", get(claim))
        .route("/health", get(|| async { "ok" }))
        .with_state(ctx)
}

pub fn snapshot(ctx: &Ctx) -> Value {
    let db = &ctx.db;
    let monitors: Vec<Value> = ctx
        .cfg
        .monitors
        .iter()
        .map(|m| {
            let probes = db.probes_since(&m.name, db::now() - m.window_secs as i64);
            let last = probes.last();
            json!({
                "name": m.name, "url": m.url, "service": m.service(), "period": m.period,
                "breach_below_pct": target_for(m), "window_secs": m.window_secs,
                "observed_uptime_pct": observed_uptime(&probes), "samples": probes.len(),
                "last_ok": last.map(|p| p.ok), "last_status": last.and_then(|p| p.status),
                "recent": probes.iter().rev().take(30).rev().map(|p| p.ok).collect::<Vec<_>>(),
            })
        })
        .collect();
    let incidents: Vec<Value> = db
        .incidents(50)
        .into_iter()
        .map(|i| {
            let purchases = db.purchases(Some(i.id));
            let spent: u64 = purchases.iter().map(|p| p.amount_uusdc).sum();
            json!({ "incident": i, "spent_uusdc": spent, "purchases": purchases.iter().map(|p| json!({
                "id": p.id, "ts": p.ts, "mode": p.mode, "miner_id": p.miner_id, "miner_name": p.miner_name,
                "amount_uusdc": p.amount_uusdc, "receipt_hash": p.receipt_hash, "signal_hash": p.signal_hash,
                "valid": p.valid, "reject_reasons": p.reject_reasons,
                "result": p.response.get("result"),
            })).collect::<Vec<_>>() })
        })
        .collect();
    let all = db.purchases(None);
    let spent: u64 = all.iter().map(|p| p.amount_uusdc).sum();
    let rejected = all.iter().filter(|p| !p.valid).count();
    let credit: f64 = db.incidents(10_000).iter().filter(|i| i.status == "claimed").filter_map(|i| i.credit_usd).sum();
    let spent_usd = spent as f64 / 1e6;
    json!({
        "wallet": ctx.tg.address(),
        "node": ctx.cfg.telegraph.node,
        "fallback_miner": ctx.cfg.telegraph.fallback_miner,
        "budget": { "per_incident_uusdc": ctx.cfg.budget.per_incident_uusdc, "per_day_uusdc": ctx.cfg.budget.per_day_uusdc,
                    "spent_today_uusdc": db.spent_today() },
        "totals": { "purchases": all.len(), "rejected": rejected, "spent_uusdc": spent, "spent_usd": spent_usd,
                    "credit_claimed_usd": credit, "ratio": if spent_usd > 0.0 { Some(credit / spent_usd) } else { None } },
        "monitors": monitors,
        "incidents": incidents,
        "events": db.events(40),
    })
}

async fn state(State(ctx): State<Arc<Ctx>>) -> Json<Value> {
    Json(snapshot(&ctx))
}

async fn claim(State(ctx): State<Arc<Ctx>>, Path(id): Path<i64>) -> Response {
    match ctx.db.incident(id).and_then(|i| i.claim_path) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], t).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        None => (StatusCode::NOT_FOUND, "no claim for this incident").into_response(),
    }
}
