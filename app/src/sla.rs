//! Published SLA credit policies (checked 2026-11-17 against the cited pages).

use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub enum Rule {
    /// (lower bound inclusive, upper bound exclusive, credit % of fees)
    Tiers(&'static [(f64, f64, f64)]),
    /// Cloudflare: credit = outage minutes / scheduled minutes (affected-customer ratio 1).
    Proportional,
    /// No published SLA: nothing is owed.
    NoSla,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub key: &'static str,
    pub name: &'static str,
    pub target_pct: f64,
    pub rule: Rule,
    pub source: &'static str,
    pub measurement: &'static str,
    pub claim_process: &'static str,
}

pub const POLICIES: &[Policy] = &[
    Policy {
        key: "github",
        name: "GitHub",
        target_pct: 99.9,
        rule: Rule::Tiers(&[(99.5, 99.9, 5.0), (99.0, 99.5, 10.0), (0.0, 99.0, 25.0)]),
        source: "https://github.com/customer-terms/github-online-services-sla",
        measurement: "calendar quarter; covers GitHub Actions, Enterprise Cloud, Packages",
        claim_process: "Written request to GitHub Support within 30 days of the end of the calendar quarter",
    },
    Policy {
        key: "cloudflare",
        name: "Cloudflare",
        target_pct: 100.0,
        rule: Rule::Proportional,
        source: "https://www.cloudflare.com/business-sla/",
        measurement: "Business plan; Service Credit = outage minutes x affected-customer ratio / scheduled minutes",
        claim_process: "Request via Cloudflare support per the Business SLA",
    },
    Policy {
        key: "vercel",
        name: "Vercel",
        // Page tiers: 99.1-99.98 -> 10%, 95-99 -> 25%, <95 -> 50%. Gaps (99.98-99.99, 99-99.1)
        // are assigned to the lower credit tier (conservative).
        target_pct: 99.99,
        rule: Rule::Tiers(&[(99.0, 99.99, 10.0), (95.0, 99.0, 25.0), (0.0, 95.0, 50.0)]),
        source: "https://vercel.com/legal/sla",
        measurement: "calendar month, Enterprise",
        claim_process: "Request via Vercel support per the Enterprise SLA",
    },
    Policy {
        key: "twilio",
        name: "Twilio",
        target_pct: 99.95,
        rule: Rule::Tiers(&[(0.0, 99.95, 10.0)]),
        source: "https://www.twilio.com/en-us/legal/service-level-agreement/twilio-apis",
        measurement: "calendar month, Services APIs",
        claim_process: "Submit within 30 days after the end of the month",
    },
    Policy {
        key: "digitalocean",
        name: "DigitalOcean",
        target_pct: 99.99,
        rule: Rule::Tiers(&[(0.0, 99.99, 100.0)]),
        source: "https://www.digitalocean.com/sla/cpu-droplets",
        measurement: "monthly billing cycle, per Droplet (credit applies to the affected Droplet's charges)",
        claim_process: "Request via DigitalOcean support ticket",
    },
    Policy {
        key: "discord",
        name: "Discord",
        target_pct: 99.9,
        rule: Rule::NoSla,
        source: "none published",
        measurement: "no published SLA; 99.9% is a reference target",
        claim_process: "No contractual credit available",
    },
];

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// "GitHub Actions" -> GitHub policy.
pub fn policy_for(service: &str) -> Option<&'static Policy> {
    let n = norm(service);
    let aliases: &[(&str, &str)] = &[("gh", "github"), ("cf", "cloudflare"), ("do", "digitalocean")];
    let n = aliases.iter().find(|(a, _)| n == *a).map(|(_, k)| k.to_string()).unwrap_or(n);
    POLICIES.iter().find(|p| n.starts_with(p.key))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Credit {
    pub credit_pct: f64,
    pub credit_usd: f64,
    pub basis: String,
}

/// Credit owed for an independently measured uptime, against `fees_usd` for the SLA period.
pub fn credit(policy: &Policy, uptime_pct: f64, fees_usd: f64) -> Option<Credit> {
    if uptime_pct >= policy.target_pct {
        return None;
    }
    let (pct, basis) = match policy.rule {
        Rule::NoSla => return None,
        Rule::Proportional => {
            let pct = ((100.0 - uptime_pct) * 1e4).round() / 1e4;
            (pct, format!("{pct}% = downtime share of scheduled minutes (Cloudflare formula)"))
        }
        Rule::Tiers(tiers) => {
            let &(lo, hi, pct) = tiers.iter().find(|(lo, hi, _)| uptime_pct >= *lo && uptime_pct < *hi)?;
            (pct, format!("{lo}% <= uptime {uptime_pct}% < {hi}% -> {pct}% of fees"))
        }
    };
    Some(Credit { credit_pct: pct, credit_usd: (fees_usd * pct).round() / 100.0, basis })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        assert_eq!(policy_for("GitHub Actions").unwrap().key, "github");
        assert_eq!(policy_for("gh").unwrap().key, "github");
        assert_eq!(policy_for("Digital Ocean").unwrap().key, "digitalocean");
        assert!(policy_for("myspace").is_none());
    }

    #[test]
    fn github_tiers() {
        let p = policy_for("GitHub").unwrap();
        assert_eq!(credit(p, 99.95, 1000.0), None);
        assert_eq!(credit(p, 99.9, 1000.0), None);
        assert_eq!(credit(p, 99.7, 1000.0).unwrap().credit_pct, 5.0);
        let c = credit(p, 99.2053, 2100.0).unwrap();
        assert_eq!((c.credit_pct, c.credit_usd), (10.0, 210.0));
        assert_eq!(credit(p, 97.0, 1000.0).unwrap().credit_usd, 250.0);
    }

    #[test]
    fn other_rules() {
        assert_eq!(credit(policy_for("Vercel").unwrap(), 99.985, 100.0).unwrap().credit_pct, 10.0);
        assert_eq!(credit(policy_for("Vercel").unwrap(), 94.0, 100.0).unwrap().credit_pct, 50.0);
        assert_eq!(credit(policy_for("Twilio").unwrap(), 99.9, 500.0).unwrap().credit_usd, 50.0);
        assert_eq!(credit(policy_for("Cloudflare").unwrap(), 99.5, 200.0).unwrap().credit_usd, 1.0);
        assert_eq!(credit(policy_for("Discord").unwrap(), 90.0, 100.0), None);
    }
}
