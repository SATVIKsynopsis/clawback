//! Vercel function: every path is rewritten here (vercel.json) and served by the shared axum router.

use tower::ServiceBuilder;
use vercel_runtime::{axum::VercelLayer, run, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    sla_miner::init_tracing();
    let app = ServiceBuilder::new().layer(VercelLayer::new()).service(sla_miner::router());
    run(app).await
}
