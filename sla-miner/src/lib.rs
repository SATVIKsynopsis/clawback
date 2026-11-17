//! sla-miner: Telegraph miner for SLA_COMPLIANCE_GENERAL.
//! Computes a provider's uptime over a past window from its public Statuspage incident history
//! and compares it to the provider's published SLA target.

pub mod app;
pub mod calc;
pub mod period;
pub mod providers;
pub mod statuspage;

pub use app::{init_tracing, router};
