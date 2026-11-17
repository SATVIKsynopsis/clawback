//! Clawback: detect SLA breaches, buy independent proof through Telegraph, validate it,
//! and draft the service-credit claim with receipts attached.

mod claim;
mod config;
mod dashboard;
mod db;
mod mock;
mod proof;
mod sla;
mod telegraph;
mod watcher;

use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Parser)]
#[command(name = "clawback", version, about = "SLA breach -> Telegraph-verified proof -> credit claim")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Watch monitors, open incidents, buy proof, draft claims, serve the dashboard.
    Run {
        #[arg(long, env = "CLAWBACK_CONFIG", default_value = "clawback.toml")]
        config: PathBuf,
    },
    /// Demo target whose /health can be switched off.
    Mock {
        #[arg(long, env = "MOCK_BIND", default_value = "0.0.0.0:8091")]
        bind: String,
    },
    /// One-off: buy and validate an SLA proof (routed, then direct fallback). Costs test USDC.
    Prove {
        #[arg(long, env = "CLAWBACK_CONFIG", default_value = "clawback.toml")]
        config: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long, default_value = "last 30 days")]
        period: String,
        /// Skip routing and ask this miner directly.
        #[arg(long)]
        miner: Option<String>,
    },
    /// Validate the config and show the wallet that would pay. Free.
    Check {
        #[arg(long, env = "CLAWBACK_CONFIG", default_value = "clawback.toml")]
        config: PathBuf,
    },
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "clawback=info,purchase=info".into()))
        .init();
}

fn telegraph(cfg: &config::Config) -> Result<telegraph::Client, String> {
    let key = std::env::var("PRIVATE_KEY").ok();
    telegraph::Client::new(
        &cfg.telegraph.node,
        &cfg.telegraph.diamond,
        &cfg.telegraph.network,
        key.as_deref(),
        Duration::from_secs(cfg.telegraph.timeout_secs),
        cfg.telegraph.max_price_uusdc,
    )
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Mock { bind } => {
            let l = tokio::net::TcpListener::bind(&bind).await.map_err(|e| format!("bind {bind}: {e}"))?;
            tracing::info!(%bind, "mock listening");
            axum::serve(l, mock::router()).await.map_err(|e| e.to_string())
        }
        Cmd::Check { config } => {
            let cfg = config::Config::load(&config)?;
            let tg = telegraph(&cfg)?;
            println!("config ok: {} monitor(s)", cfg.monitors.len());
            println!("node: {}  diamond: {}", cfg.telegraph.node, cfg.telegraph.diamond);
            println!("wallet: {}", tg.address().unwrap_or_else(|| "none (PRIVATE_KEY unset: proofs will be refused)".into()));
            println!("budget: {} uUSDC/incident, {} uUSDC/day", cfg.budget.per_incident_uusdc, cfg.budget.per_day_uusdc);
            for m in &cfg.monitors {
                println!("- {}: {} -> ask '{}' for '{}', breach below {}%", m.name, m.url, m.service(), m.period, watcher::target_for(m));
            }
            Ok(())
        }
        Cmd::Prove { config, service, period, miner } => {
            let cfg = config::Config::load(&config)?;
            let tg = telegraph(&cfg)?;
            let db = db::Db::open(&cfg.watcher.db_path).map_err(|e| e.to_string())?;
            let budget = proof::Budget { db: &db, incident_id: None, per_incident: cfg.budget.per_incident_uusdc, per_day: cfg.budget.per_day_uusdc };
            let res = match miner {
                Some(m) => {
                    // direct only: treat the given miner as the only attempt
                    let paid = tg
                        .ask_direct(&m, proof::INTENT, serde_json::json!({"service": service, "period": period}), |p| budget.check(p))
                        .await
                        .map_err(|e| e.to_string())?;
                    let verdict = proof::validate(&service, &paid.body);
                    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                        "valid": verdict.is_ok(), "reject_reasons": verdict.err(),
                        "response": paid.body, "receipt": paid.receipt })).unwrap());
                    return Ok(());
                }
                None => proof::obtain(&tg, &db, &budget, None, &service, &period, cfg.telegraph.fallback_miner.as_deref()).await,
            };
            match res {
                Ok(p) => {
                    println!("{}", serde_json::to_string_pretty(&p).unwrap());
                    Ok(())
                }
                Err(f) => Err(format!("no validated proof:\n  {}", f.reasons.join("\n  "))),
            }
        }
        Cmd::Run { config } => {
            let cfg = config::Config::load(&config)?;
            let tg = telegraph(&cfg)?;
            if tg.address().is_none() {
                tracing::warn!("PRIVATE_KEY not set: incidents will be detected but proofs refused (no claims)");
            }
            let db = Arc::new(db::Db::open(&cfg.watcher.db_path).map_err(|e| e.to_string())?);
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(cfg.watcher.probe_timeout_secs))
                .user_agent(concat!("clawback-watcher/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|e| e.to_string())?;
            let bind = cfg.dashboard.bind.clone();
            let ctx = Arc::new(watcher::Ctx { cfg, db, tg: Arc::new(tg), http });
            for m in ctx.cfg.monitors.clone() {
                tokio::spawn(watcher::run_monitor(ctx.clone(), m));
            }
            let l = tokio::net::TcpListener::bind(&bind).await.map_err(|e| format!("bind {bind}: {e}"))?;
            tracing::info!(%bind, monitors = ctx.cfg.monitors.len(), "clawback running; dashboard up");
            axum::serve(l, dashboard::router(ctx)).await.map_err(|e| e.to_string())
        }
    }
}
