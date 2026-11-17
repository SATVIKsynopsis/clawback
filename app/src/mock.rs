//! Demo target: /health is 200 until you make it fail.
//!   curl -X POST localhost:8091/fail      curl -X POST localhost:8091/recover

use axum::{extract::State, http::StatusCode, response::Html, routing::{get, post}, Json, Router};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub fn router() -> Router {
    let healthy = Arc::new(AtomicBool::new(true));
    let set = |v: bool| move |State(h): State<Arc<AtomicBool>>| async move {
        h.store(v, Ordering::SeqCst);
        tracing::warn!(healthy = v, "mock state changed");
        Json(json!({ "healthy": v }))
    };
    Router::new()
        .route(
            "/health",
            get(|State(h): State<Arc<AtomicBool>>| async move {
                if h.load(Ordering::SeqCst) {
                    (StatusCode::OK, Json(json!({ "status": "ok" })))
                } else {
                    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "down", "reason": "mock outage" })))
                }
            }),
        )
        .route("/fail", post(set(false)))
        .route("/recover", post(set(true)))
        .route(
            "/toggle",
            post(|State(h): State<Arc<AtomicBool>>| async move {
                let v = !h.fetch_xor(true, Ordering::SeqCst);
                Json(json!({ "healthy": v }))
            }),
        )
        .route(
            "/",
            get(|State(h): State<Arc<AtomicBool>>| async move {
                let up = h.load(Ordering::SeqCst);
                Html(format!(
                    "<!doctype html><meta name=viewport content='width=device-width'><title>Mock service</title>\
                     <body style='font-family:system-ui;padding:24px'><h1>Mock service: {}</h1>\
                     <form method=post action=/fail><button>Make /health fail</button></form><br>\
                     <form method=post action=/recover><button>Recover</button></form>",
                    if up { "healthy" } else { "DOWN" }
                ))
            }),
        )
        .with_state(healthy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn toggles() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router()).await.unwrap() });
        let c = reqwest::Client::new();
        assert_eq!(c.get(format!("http://{a}/health")).send().await.unwrap().status(), 200);
        c.post(format!("http://{a}/fail")).send().await.unwrap();
        assert_eq!(c.get(format!("http://{a}/health")).send().await.unwrap().status(), 503);
        c.post(format!("http://{a}/toggle")).send().await.unwrap();
        assert_eq!(c.get(format!("http://{a}/health")).send().await.unwrap().status(), 200);
    }
}
