//! sla-miner HTTP server. Binds 0.0.0.0:$PORT (default 8080). On Vercel, the Rust builder runs
//! this binary in standalone-server mode and sets PORT.

#[tokio::main]
async fn main() {
    sla_miner::init_tracing();
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    tracing::info!(port, "sla-miner listening");
    axum::serve(listener, sla_miner::router()).await.expect("server");
}
