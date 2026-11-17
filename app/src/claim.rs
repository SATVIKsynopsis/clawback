//! Credit computation and claim email (.eml always written; sent via SMTP if configured).

use crate::config::{Config, MonitorCfg};
use crate::db::{iso, Incident, Purchase};
use crate::proof::SlaProof;
use crate::sla::{self, Credit};
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::{json, Value};
use std::fmt::Write as _;

/// Soft-wrap lines at `width` on spaces, keeping the line's indent for continuations.
/// Long unbreakable tokens (hashes) are kept whole.
pub fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.len() <= width {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        let cont = format!("{indent}  ");
        let mut cur = String::new();
        for word in line.trim_start().split(' ') {
            let prefix = if cur.is_empty() { if out.ends_with('\n') || out.is_empty() { indent.as_str() } else { cont.as_str() } } else { "" };
            if !cur.is_empty() && cur.len() + 1 + word.len() > width {
                out.push_str(&cur);
                out.push('\n');
                cur = format!("{cont}{word}");
            } else if cur.is_empty() {
                cur = format!("{prefix}{word}");
            } else {
                cur.push(' ');
                cur.push_str(word);
            }
        }
        out.push_str(&cur);
        out.push('\n');
    }
    out
}

pub struct Draft {
    pub path: String,
    pub credit: Option<Credit>,
    pub summary: String,
}

fn outage_lines(outages: &Value, max: usize) -> Vec<String> {
    outages
        .as_array()
        .map(|a| {
            a.iter()
                .take(max)
                .map(|o| {
                    format!(
                        "   - {} -> {} ({} min) {}",
                        o["start"].as_str().unwrap_or("?"),
                        o["end"].as_str().unwrap_or("?"),
                        o["minutes"],
                        o["incidents"].as_array().map(|i| i.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default()
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn render(cfg: &Config, inc: &Incident, m: &MonitorCfg, proof: &SlaProof, windows: &[(i64, i64)], purchases: &[Purchase]) -> (String, String, Option<Credit>) {
    let policy = sla::policy_for(&m.provider);
    let credit = policy.and_then(|p| sla::credit(p, proof.uptime_pct, m.fees_usd));
    let provider = policy.map_or(m.provider.as_str(), |p| p.name);
    let subject = format!("SLA service credit request: {} ({}, measured uptime {}%)", proof.service, proof.period, proof.uptime_pct);
    let node = cfg.telegraph.node.trim_end_matches('/');

    let mut b = String::new();
    let _ = writeln!(b, "Hello {provider} Support,\n");
    match (&credit, policy) {
        (Some(c), Some(p)) => {
            let _ = writeln!(
                b,
                "We request a service credit under your published SLA ({}) for {}.\n",
                p.source, proof.service
            );
            if !m.account_ref.is_empty() {
                let _ = writeln!(b, "Account: {}", m.account_ref);
            }
            let _ = writeln!(b, "Period measured:      {}", proof.period);
            let _ = writeln!(b, "SLA commitment:       {}% ({})", p.target_pct, p.measurement);
            let _ = writeln!(b, "Measured uptime:      {}% (independent, see evidence 2)", proof.uptime_pct);
            let _ = writeln!(b, "Credit tier:          {}", c.basis);
            let _ = writeln!(b, "Credit requested:     {}% of applicable fees (${:.2}) = ${:.2}\n", c.credit_pct, m.fees_usd, c.credit_usd);
        }
        _ => {
            let _ = writeln!(
                b,
                "{} measured {}% uptime over {} (target {}%). {}\n",
                proof.service,
                proof.uptime_pct,
                proof.period,
                proof.sla_target_pct,
                policy.map_or("We have no credit table for this provider.", |p| p.claim_process)
            );
        }
    }

    let _ = writeln!(b, "EVIDENCE\n");
    let _ = writeln!(b, "1. Our monitoring: {} ({})", m.name, m.url);
    let _ = writeln!(b, "   Observed uptime during the incident window: {}% (incident opened {})", inc.observed_uptime_pct, iso(inc.opened_at));
    for (s, e) in windows {
        let _ = writeln!(b, "   - down {} -> {} ({} min)", iso(*s), iso(*e), (e - s) as f64 / 60.0);
    }
    let _ = writeln!(b, "\n2. Independent verification via Telegraph (intent SLA_COMPLIANCE_GENERAL),");
    let _ = writeln!(b, "   computed from your own public status page incident history:");
    let _ = writeln!(b, "   Result:       {} uptime {}% vs target {}% -> {}", proof.service, proof.uptime_pct, proof.sla_target_pct, if proof.compliant { "met" } else { "NOT met" });
    let _ = writeln!(b, "   Answered by:  {} (miner {}, {} ask)", proof.miner_name, proof.miner_id, proof.mode);
    let _ = writeln!(b, "   Receipt hash:\n     {}", proof.receipt_hash);
    let _ = writeln!(b, "   Receipt/signal can be looked up at {node}/engine/v1/signal/<signal hash>");
    let _ = writeln!(b, "   Signal hash:\n     {}", proof.signal_hash);
    if let Some(src) = &proof.sla_source {
        let _ = writeln!(b, "   SLA source:   {src}");
    }
    if let Some(o) = &proof.outages {
        let lines = outage_lines(o, 15);
        if !lines.is_empty() {
            let _ = writeln!(b, "   Provider-declared outages in the period:");
            for l in lines {
                let _ = writeln!(b, "{l}");
            }
        }
    }
    let rejected: Vec<&Purchase> = purchases.iter().filter(|p| !p.valid).collect();
    if !rejected.is_empty() {
        let _ = writeln!(b, "\n   Answers we rejected during validation (not relied on):");
        for p in rejected {
            let _ = writeln!(b, "   - {} ({}): {}\n     receipt {}", p.miner_name, p.mode, p.reject_reasons.join("; "), p.receipt_hash);
        }
    }
    if let Some(p) = policy {
        let _ = writeln!(b, "\nClaim process: {}", p.claim_process);
    }
    let _ = writeln!(b, "Full Telegraph responses and receipts are attached (telegraph-evidence.json).\n\nRegards,\n{}", cfg.claim.from);
    (subject, b, credit)
}

pub async fn draft(cfg: &Config, inc: &Incident, m: &MonitorCfg, proof: &SlaProof, windows: &[(i64, i64)], purchases: &[Purchase]) -> Result<Draft, String> {
    let (subject, body, credit) = render(cfg, inc, m, proof, windows, purchases);
    let evidence = json!({
        "incident": inc, "proof": proof, "observed_outage_windows": windows,
        "telegraph_purchases": purchases,
    });
    let from: Mailbox = cfg.claim.from.parse().map_err(|e| format!("claim.from '{}': {e}", cfg.claim.from))?;
    let to: Mailbox = m.claim_to.parse().map_err(|e| format!("claim_to '{}': {e}", m.claim_to))?;
    let msg = Message::builder()
        .from(from)
        .to(to)
        .subject(&subject)
        .multipart(
            MultiPart::mixed()
                // body is wrapped below 76 cols, so it goes out as plain 7bit and
                // receipt/signal hashes stay intact and greppable in the raw .eml
                .singlepart(SinglePart::plain(wrap(&body, 74)))
                .singlepart(
                Attachment::new("telegraph-evidence.json".into())
                    .body(serde_json::to_string_pretty(&evidence).unwrap(), ContentType::parse("application/json").unwrap()),
            ),
        )
        .map_err(|e| format!("build email: {e}"))?;

    std::fs::create_dir_all(&cfg.claim.out_dir).map_err(|e| format!("create {}: {e}", cfg.claim.out_dir))?;
    let path = format!("{}/incident-{}.eml", cfg.claim.out_dir.trim_end_matches('/'), inc.id);
    std::fs::write(&path, msg.formatted()).map_err(|e| format!("write {path}: {e}"))?;

    let mut sent = false;
    if let Some(smtp) = &cfg.claim.smtp {
        let password = std::env::var(&smtp.password_env).map_err(|_| format!("SMTP configured but ${} is not set", smtp.password_env))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
            .map_err(|e| format!("smtp {}: {e}", smtp.host))?
            .port(smtp.port)
            .credentials(lettre::transport::smtp::authentication::Credentials::new(smtp.username.clone(), password))
            .build();
        transport.send(msg).await.map_err(|e| format!("smtp send failed (draft kept at {path}): {e}"))?;
        sent = true;
    }

    let summary = match &credit {
        Some(c) => format!(
            "claim {} for ${:.2} ({}% of ${:.2}) based on independent uptime {}%: {path}",
            if sent { "sent" } else { "drafted" },
            c.credit_usd,
            c.credit_pct,
            m.fees_usd,
            proof.uptime_pct
        ),
        None => format!("SLA missed ({}% < {}%) but no credit applies for {}; notice drafted: {path}", proof.uptime_pct, proof.sla_target_pct, m.provider),
    };
    Ok(Draft { path, credit, summary })
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_under_76_keeping_tokens() {
        let h = format!("0x{}", "a".repeat(64));
        let text = format!("   {} {} {}\nshort\n", "word ".repeat(30).trim(), h, "tail");
        let w = wrap(&text, 74);
        assert!(w.lines().all(|l| l.len() < 76), "{w}");
        assert!(w.contains(&h));
        assert!(w.lines().skip(1).take(1).all(|l| l.starts_with("     ")));
    }
}
