//! Supported Statuspage-hosted providers and their published SLA targets.
//! Every `sla_target_pct` below was checked against the cited page on 2026-11-17.

#[derive(Clone, Copy, Debug)]
pub struct Provider {
    pub key: &'static str,
    pub name: &'static str,
    pub base: &'static str,
    pub aliases: &'static [&'static str],
    pub sla_target_pct: f64,
    /// false = provider publishes no SLA; target is a reference value, not a contract.
    pub sla_published: bool,
    pub sla_source: &'static str,
    pub sla_note: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    // https://github.com/customer-terms/github-online-services-sla
    // "GitHub commits to maintain at least 99.9% Uptime" (calendar quarter; Actions, Enterprise Cloud, Packages)
    Provider {
        key: "github",
        name: "GitHub",
        base: "https://www.githubstatus.com",
        aliases: &["github", "gh", "githubcom", "githubstatus", "githubstatuscom"],
        sla_target_pct: 99.9,
        sla_published: true,
        sla_source: "https://github.com/customer-terms/github-online-services-sla",
        sla_note: "99.9% per calendar quarter; covers GitHub Actions, Enterprise Cloud, Packages",
    },
    // https://www.cloudflare.com/business-sla/  §1.1 "100% Uptime"
    Provider {
        key: "cloudflare",
        name: "Cloudflare",
        base: "https://www.cloudflarestatus.com",
        aliases: &["cloudflare", "cf", "cloudflarecom", "cloudflarestatus"],
        sla_target_pct: 100.0,
        sla_published: true,
        sla_source: "https://www.cloudflare.com/business-sla/",
        sla_note: "100% uptime (Business plan SLA)",
    },
    // https://vercel.com/legal/sla  "available 99.99% of the time" (calendar month, Enterprise)
    Provider {
        key: "vercel",
        name: "Vercel",
        base: "https://www.vercel-status.com",
        aliases: &["vercel", "vercelcom", "zeit"],
        sla_target_pct: 99.99,
        sla_published: true,
        sla_source: "https://vercel.com/legal/sla",
        sla_note: "99.99% per calendar month (Enterprise)",
    },
    // https://www.twilio.com/en-us/legal/service-level-agreement/twilio-apis  99.95% monthly (Services APIs)
    Provider {
        key: "twilio",
        name: "Twilio",
        base: "https://status.twilio.com",
        aliases: &["twilio", "twiliocom"],
        sla_target_pct: 99.95,
        sla_published: true,
        sla_source: "https://www.twilio.com/en-us/legal/service-level-agreement/twilio-apis",
        sla_note: "99.95% monthly availability for Twilio Services APIs",
    },
    // https://www.digitalocean.com/sla/cpu-droplets  "Monthly Uptime Percentage of 99.99%"
    Provider {
        key: "digitalocean",
        name: "DigitalOcean",
        base: "https://status.digitalocean.com",
        aliases: &["digitalocean", "do", "digitaloceancom", "digital ocean"],
        sla_target_pct: 99.99,
        sla_published: true,
        sla_source: "https://www.digitalocean.com/sla/cpu-droplets",
        sla_note: "99.99% monthly per Droplet (CPU Droplet SLA)",
    },
    // Discord publishes no SLA. 99.9% is a reference target only; sla_published=false says so.
    Provider {
        key: "discord",
        name: "Discord",
        base: "https://discordstatus.com",
        aliases: &["discord", "discordapp", "discordcom"],
        sla_target_pct: 99.9,
        sla_published: false,
        sla_source: "none published",
        sla_note: "Discord publishes no SLA; 99.9% is a reference target, not a contractual commitment",
    },
];

fn norm(s: &str) -> String {
    s.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Resolve a service string to (provider, optional component).
/// "GitHub" -> (GitHub, None); "github.com" -> (GitHub, None); "GitHub Actions" -> (GitHub, Some("Actions")).
pub fn lookup(service: &str) -> Option<(&'static Provider, Option<String>)> {
    let full = norm(service);
    if let Some(p) = PROVIDERS.iter().find(|p| p.aliases.iter().any(|a| norm(a) == full)) {
        return Some((p, None));
    }
    // "<provider> <component...>": try the longest word prefix that names a provider.
    let words: Vec<&str> = service.split_whitespace().collect();
    for n in (1..words.len()).rev() {
        let head = norm(&words[..n].join(" "));
        if let Some(p) = PROVIDERS.iter().find(|p| p.aliases.iter().any(|a| norm(a) == head)) {
            let rest = words[n..].join(" ");
            return Some((p, Some(rest)));
        }
    }
    None
}

pub fn supported() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases() {
        for s in ["GitHub", "gh", "github.com", " GITHUB "] {
            let (p, c) = lookup(s).unwrap();
            assert_eq!((p.key, c), ("github", None), "{s}");
        }
        assert_eq!(lookup("Digital Ocean").unwrap().0.key, "digitalocean");
        assert!(lookup("myspace").is_none());
    }

    #[test]
    fn component_suffix() {
        let (p, c) = lookup("GitHub Actions").unwrap();
        assert_eq!((p.key, c.as_deref()), ("github", Some("Actions")));
        let (p, c) = lookup("github.com Git Operations").unwrap();
        assert_eq!((p.key, c.as_deref()), ("github", Some("Git Operations")));
    }

    #[test]
    fn all_targets_are_percentages() {
        for p in PROVIDERS {
            assert!(p.sla_target_pct > 90.0 && p.sla_target_pct <= 100.0);
            assert!(p.base.starts_with("https://"));
        }
    }
}
