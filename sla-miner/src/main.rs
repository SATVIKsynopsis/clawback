//! Local / Docker server. On Vercel the same router runs from api/sla.rs.

#[tokio::main]
async fn main() {
    sla_miner::init_tracing();
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    tracing::info!(port, "sla-miner listening");
    axum::serve(listener, sla_miner::router()).await.expect("server");
}
