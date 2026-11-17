//! clawback.toml + environment overrides.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub telegraph: TelegraphCfg,
    pub budget: BudgetCfg,
    #[serde(default)]
    pub watcher: WatcherCfg,
    #[serde(default)]
    pub dashboard: DashboardCfg,
    pub claim: ClaimCfg,
    #[serde(rename = "monitor", default)]
    pub monitors: Vec<MonitorCfg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegraphCfg {
    /// Node HTTP API, e.g. https://devnode.telegraphprotocol.com
    pub node: String,
    /// The only payTo we will ever sign for (the Telegraph Diamond).
    pub diamond: String,
    #[serde(default = "default_network")]
    pub network: String,
    /// Miner id for the direct-ask fallback when the routed answer fails validation.
    pub fallback_miner: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Refuse any single request priced above this (uUSDC, 6 decimals).
    #[serde(default = "default_max_price")]
    pub max_price_uusdc: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BudgetCfg {
    pub per_incident_uusdc: u64,
    pub per_day_uusdc: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatcherCfg {
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_db")]
    pub db_path: String,
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_secs: u64,
}

impl Default for WatcherCfg {
    fn default() -> Self {
        Self { interval_secs: default_interval(), db_path: default_db(), probe_timeout_secs: default_probe_timeout() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DashboardCfg {
    #[serde(default = "default_bind")]
    pub bind: String,
}

impl Default for DashboardCfg {
    fn default() -> Self {
        Self { bind: default_bind() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimCfg {
    #[serde(default = "default_claims_dir")]
    pub out_dir: String,
    pub from: String,
    pub smtp: Option<SmtpCfg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpCfg {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    pub username: String,
    /// Name of the env var holding the SMTP password (never put secrets in the TOML).
    #[serde(default = "default_smtp_password_env")]
    pub password_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorCfg {
    /// Display name, e.g. "Checkout -> GitHub API"
    pub name: String,
    /// URL probed every interval; any 2xx is "up".
    pub url: String,
    /// Service asked from Telegraph (SLA_COMPLIANCE_GENERAL `service`), e.g. "GitHub".
    pub provider: String,
    /// Optional component, appended to the service ("GitHub Actions").
    pub component: Option<String>,
    /// Period asked from Telegraph, e.g. "last 30 days".
    #[serde(default = "default_period")]
    pub period: String,
    /// Observed-uptime window for breach detection.
    #[serde(default = "default_window")]
    pub window_secs: u64,
    #[serde(default = "default_min_samples")]
    pub min_samples: usize,
    /// Open an incident when observed uptime drops below this. Defaults to the provider's SLA target.
    pub breach_below_pct: Option<f64>,
    /// Fees paid for the SLA-covered service in the SLA's measurement period (credit base), USD.
    pub fees_usd: f64,
    /// Where the claim goes (provider support address).
    pub claim_to: String,
    /// Customer account reference included in the claim.
    #[serde(default)]
    pub account_ref: String,
}

impl MonitorCfg {
    pub fn service(&self) -> String {
        match &self.component {
            Some(c) if !c.trim().is_empty() => format!("{} {}", self.provider, c.trim()),
            _ => self.provider.clone(),
        }
    }
}

fn default_network() -> String {
    "eip155:11155111".into()
}
fn default_timeout() -> u64 {
    20
}
fn default_max_price() -> u64 {
    50_000
}
fn default_interval() -> u64 {
    10
}
fn default_probe_timeout() -> u64 {
    5
}
fn default_db() -> String {
    "clawback.db".into()
}
fn default_bind() -> String {
    "0.0.0.0:8090".into()
}
fn default_claims_dir() -> String {
    "claims".into()
}
fn default_smtp_port() -> u16 {
    587
}
fn default_smtp_password_env() -> String {
    "SMTP_PASSWORD".into()
}
fn default_period() -> String {
    "last 30 days".into()
}
fn default_window() -> u64 {
    120
}
fn default_min_samples() -> usize {
    6
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        cfg.apply_env();
        cfg.check()?;
        Ok(cfg)
    }

    /// Env wins over the file, so docker compose / CI can tweak without editing TOML.
    pub fn apply_env(&mut self) {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        if let Some(v) = get("TELEGRAPH_NODE").or_else(|| get("NODE")) {
            self.telegraph.node = v;
        }
        if let Some(v) = get("TELEGRAPH_DIAMOND").or_else(|| get("DIAMOND")) {
            self.telegraph.diamond = v;
        }
        if let Some(v) = get("CLAWBACK_FALLBACK_MINER") {
            self.telegraph.fallback_miner = Some(v);
        }
        if let Some(v) = get("CLAWBACK_BUDGET_PER_INCIDENT_UUSDC").and_then(|v| v.parse().ok()) {
            self.budget.per_incident_uusdc = v;
        }
        if let Some(v) = get("CLAWBACK_BUDGET_PER_DAY_UUSDC").and_then(|v| v.parse().ok()) {
            self.budget.per_day_uusdc = v;
        }
        if let Some(v) = get("CLAWBACK_DB") {
            self.watcher.db_path = v;
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if !self.telegraph.diamond.starts_with("0x") || self.telegraph.diamond.len() != 42 {
            return Err("telegraph.diamond must be a 0x-prefixed address".into());
        }
        for m in &self.monitors {
            if m.window_secs < self.watcher.interval_secs * m.min_samples as u64 {
                return Err(format!(
                    "monitor '{}': window_secs {} cannot hold min_samples {} at interval {}s",
                    m.name, m.window_secs, m.min_samples, self.watcher.interval_secs
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub fn test_config() -> Config {
    toml::from_str(
        r#"
        [telegraph]
        node = "http://127.0.0.1:1"
        diamond = "0x72a70e949362670Bf8bceAd24eFC5b8f950aa08b"
        fallback_miner = "7410311"
        [budget]
        per_incident_uusdc = 30000
        per_day_uusdc = 100000
        [claim]
        from = "ops@example.com"
        [[monitor]]
        name = "demo"
        url = "http://127.0.0.1:1/health"
        provider = "GitHub"
        fees_usd = 2100.0
        claim_to = "support@github.example"
        "#,
    )
    .unwrap()
}
