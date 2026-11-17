//! HTTP layer: the axum router served by src/main.rs (locally, in Docker, and on Vercel).

use crate::{calc, period, providers, statuspage};

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Instant};

const METHOD_COMPONENT: &str = "Source: the provider's own public Statuspage incident history (major/critical impact only). \
Downtime = time any matching component was marked partial_outage or major_outage, reconstructed from each incident's \
update log (incident_updates[].affected_components); degraded_performance is not downtime. Incidents with no component \
data fall back to their whole window (started_at -> monitoring_at). Spans are clipped to the window and overlaps merged. \
uptime_pct = 1 - downtime/window.";
const METHOD_WINDOW: &str = "Source: the provider's own public Statuspage incident history (major/critical impact only). \
Downtime = each incident's whole window, started_at (fallback created_at) -> monitoring_at (fallback resolved_at, then now), \
clipped to the window, overlaps merged. Conservative upper bound. uptime_pct = 1 - downtime/window.";

#[derive(Deserialize)]
struct SlaQuery {
    service: Option<String>,
    period: Option<String>,
    days: Option<String>,
    component: Option<String>,
    basis: Option<String>,
}

#[derive(Serialize)]
struct SlaResponse {
    // --- SLA_COMPLIANCE_GENERAL fields ---
    service: String,
    uptime_pct: f64,
    sla_target_pct: f64,
    compliant: bool,
    period: String,
    // --- evidence ---
    provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    component: Option<String>,
    window_start: String,
    window_end: String,
    basis: calc::Basis,
    downtime_minutes: f64,
    incidents_counted: usize,
    incidents_window_fallback: usize,
    /// cross-check: same data measured with the conservative incident-window basis
    uptime_pct_incident_window: f64,
    outages: Vec<calc::Outage>,
    coverage_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_note: Option<String>,
    sla_published: bool,
    sla_source: &'static str,
    sla_note: &'static str,
    source: String,
    method: &'static str,
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    extra: serde_json::Value,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: json!({}) }
    }
    fn with(mut self, extra: serde_json::Value) -> Self {
        self.extra = extra;
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message, "code": self.code });
        if let (Some(b), Some(e)) = (body.as_object_mut(), self.extra.as_object()) {
            b.extend(e.clone());
        }
        (self.status, [(header::CACHE_CONTROL, NO_STORE)], Json(body)).into_response()
    }
}

/// Successful answers are cached at the CDN for 5 minutes (status pages change slowly);
/// errors are never cached.
pub const CDN_CACHE: &str = "public, max-age=0, s-maxage=300, stale-while-revalidate=60";
const NO_STORE: &str = "no-store";

struct AppState {
    sp: statuspage::Client,
}

async fn sla(State(st): State<Arc<AppState>>, Query(q): Query<SlaQuery>) -> Response {
    let t0 = Instant::now();
    let result = sla_inner(&st, &q).await;
    match &result {
        Ok(r) => tracing::info!(
            service = %r.service, period = %r.period, uptime_pct = r.uptime_pct, compliant = r.compliant,
            incidents = r.incidents_counted, coverage_complete = r.coverage_complete,
            ms = t0.elapsed().as_millis() as u64, "sla ok"
        ),
        Err(e) => tracing::warn!(
            service = q.service.as_deref().unwrap_or(""), period = q.period.as_deref().unwrap_or(""),
            status = e.status.as_u16(), code = e.code, error = %e.message,
            ms = t0.elapsed().as_millis() as u64, "sla error"
        ),
    }
    match result {
        Ok(r) => ([(header::CACHE_CONTROL, CDN_CACHE)], Json(r)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn sla_inner(st: &AppState, q: &SlaQuery) -> Result<SlaResponse, ApiError> {
    let service = q.service.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "missing_service", "query param 'service' is required")
            .with(json!({ "supported_services": providers::supported() }))
    })?;
    let (p, suffix_component) = providers::lookup(service).ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "unsupported_service", format!("unsupported service '{service}'"))
            .with(json!({ "supported_services": providers::supported() }))
    })?;
    let days = period::resolve_days(q.days.as_deref(), q.period.as_deref())
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "bad_period", m))?;
    let basis = match q.basis.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => calc::Basis::ComponentStatus,
        Some(b) => calc::Basis::parse(b).ok_or_else(|| {
            ApiError::new(StatusCode::BAD_REQUEST, "bad_basis", format!("unknown basis '{b}'; use component_status or incident_window"))
        })?,
    };

    // component: explicit param wins over "GitHub Actions"-style suffix. Validate against the live list.
    let requested_component = q.component.clone().filter(|c| !c.trim().is_empty()).or(suffix_component);
    let component = match requested_component {
        None => None,
        Some(c) => {
            let names = st.sp.components(p.base).await.map_err(upstream)?;
            let lc = c.trim().to_lowercase();
            let matched: Vec<&String> = names.iter().filter(|n| n.to_lowercase().contains(&lc)).collect();
            match matched.as_slice() {
                [] => {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "unknown_component",
                        format!("'{c}' matches no {} status-page component", p.name),
                    )
                    .with(json!({ "components": names })));
                }
                [one] => Some(((*one).clone(), c.trim().to_string())),
                _ => Some((c.trim().to_string(), c.trim().to_string())),
            }
        }
    };

    let end = Utc::now();
    let start = end - Duration::days(days);
    let fetched = st.sp.incidents_since(p.base, start).await.map_err(upstream)?;
    let filter = component.as_ref().map(|(_, f)| f.as_str());
    let comp = calc::compute(&fetched.incidents, start, end, filter, basis);
    let window = match basis {
        calc::Basis::IncidentWindow => comp.uptime_pct,
        calc::Basis::ComponentStatus => calc::compute(&fetched.incidents, start, end, filter, calc::Basis::IncidentWindow).uptime_pct,
    };

    let display = match &component {
        Some((name, _)) => format!("{} {}", p.name, name),
        None => p.name.to_string(),
    };
    Ok(SlaResponse {
        service: display,
        uptime_pct: comp.uptime_pct,
        sla_target_pct: p.sla_target_pct,
        compliant: comp.uptime_pct >= p.sla_target_pct,
        period: format!("last {days} days"),
        provider: p.name,
        component: component.map(|(name, _)| name),
        window_start: start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        window_end: end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        basis,
        downtime_minutes: (comp.downtime.num_seconds() as f64 / 6.0).round() / 10.0,
        incidents_counted: comp.incidents_counted,
        incidents_window_fallback: comp.incidents_window_fallback,
        uptime_pct_incident_window: window,
        outages: comp.outages,
        coverage_complete: fetched.coverage_complete,
        coverage_note: fetched.coverage_note,
        sla_published: p.sla_published,
        sla_source: p.sla_source,
        sla_note: p.sla_note,
        source: format!(
            "{}/api/v2/incidents.json{}",
            p.base,
            if fetched.history_pages > 0 {
                format!(" + /history.json ({} pages, {} incident lookups)", fetched.history_pages, fetched.detail_fetches)
            } else {
                String::new()
            }
        ),
        method: match basis {
            calc::Basis::ComponentStatus => METHOD_COMPONENT,
            calc::Basis::IncidentWindow => METHOD_WINDOW,
        },
    })
}

fn upstream(e: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "upstream_error", format!("status page unavailable: {e}"))
}

async fn index() -> Response {
    let body = Json(json!({
        "name": "sla-miner",
        "intent": "SLA_COMPLIANCE_GENERAL",
        "usage": "GET /sla?service=GitHub&period=last%2030%20days[&component=Actions][&basis=component_status|incident_window]",
        "providers": providers::PROVIDERS.iter().map(|p| json!({
            "key": p.key, "name": p.name, "status_page": p.base, "sla_target_pct": p.sla_target_pct,
            "sla_published": p.sla_published, "sla_source": p.sla_source,
        })).collect::<Vec<_>>(),
        "method": { "component_status (default)": METHOD_COMPONENT, "incident_window": METHOD_WINDOW },
    }));
    ([(header::CACHE_CONTROL, "public, s-maxage=3600")], body).into_response()
}

async fn health() -> Response {
    ([(header::CACHE_CONTROL, NO_STORE)], Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))).into_response()
}

async fn fallback() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown path; try GET /sla?service=GitHub&period=last%2030%20days")
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("sla-miner/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("http client")
}

/// All routes. `/api/sla` is kept as an alias of `/sla`.
pub fn router() -> Router {
    let state = Arc::new(AppState { sp: statuspage::Client::new(http_client()) });
    Router::new()
        .route("/", get(index))
        .route("/sla", get(sla))
        .route("/api/sla", get(sla))
        .route("/health", get(health))
        .fallback(fallback)
        .with_state(state)
}

/// JSON logs to stdout (Vercel captures stdout as function logs).
pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "sla_miner=info".into()),
        )
        .try_init();
}
