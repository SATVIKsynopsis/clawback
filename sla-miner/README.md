# sla-miner

A [Telegraph](https://anjoman.ai/docs) miner for **`SLA_COMPLIANCE_GENERAL`**. Ask it "was GitHub
within its SLA over the last 30 days?" and it answers from the provider's **own public status-page
incident history**, compared with the provider's **published SLA**.

```
GET /sla?service=GitHub&period=last%2030%20days
```
```json
{
  "service": "GitHub", "uptime_pct": 99.2053, "sla_target_pct": 99.9, "compliant": false,
  "period": "last 30 days",
  "basis": "component_status", "downtime_minutes": 343.3, "incidents_counted": 9,
  "uptime_pct_incident_window": 97.5822,
  "outages": [{ "start": "2026-11-04T09:53:...Z", "end": "2026-11-04T11:30:...Z", "minutes": 97.1,
                "incidents": ["Incident with Pull Requests, Git Operations and Issues [Pull Requests]"] }, "..."],
  "coverage_complete": true, "sla_published": true,
  "sla_source": "https://github.com/customer-terms/github-online-services-sla",
  "source": "https://www.githubstatus.com/api/v2/incidents.json", "method": "..."
}
```

> Full README (problem, quickstart, method, limitations, costs) lands in Phase E. Method and a
> hand-checked comparison are in [VERIFICATION.md](VERIFICATION.md).

## Run locally
```bash
cargo run -p sla-miner                       # from the repo root; listens on $PORT (default 8080)
curl 'localhost:8080/sla?service=GitHub%20Actions&period=last%20week'
cargo test -p sla-miner
```

## Deploy (Vercel, Rust runtime)
The same axum router runs as one Vercel function (`api/sla.rs`, `vercel_runtime` + `VercelLayer`);
`vercel.json` rewrites every path to it. Answers carry `Cache-Control: s-maxage=300` so the CDN
caches them for 5 minutes; errors are `no-store`.
```bash
# Vercel project settings: Root Directory = sla-miner (include files outside root: on)
npx vercel link            # from sla-miner/
npx vercel deploy --prod
npx vercel domains add sla.tracelet.xyz
```
Fallback host: `render.yaml` (Docker, free tier sleeps when idle).

## Docker
```bash
docker build -f sla-miner/Dockerfile -t sla-miner .   # from the repo root
docker run -p 8080:8080 sla-miner
```

## Parameters
| Param | Required | Example |
|---|---|---|
| `service` | yes | `GitHub`, `gh`, `github.com`, `GitHub Actions`, `Cloudflare`, `Vercel`, `Twilio`, `DigitalOcean`, `Discord` |
| `period` | no (default 30 days) | `last 30 days`, `last week`, `past 2 months`, `30d` (1–90 days) |
| `days` | no | `14` (overrides `period`) |
| `component` | no | `Actions`, `Git Operations` (validated against the live component list) |
| `basis` | no | `component_status` (default) or `incident_window` |

Errors are JSON `{ "error", "code", ... }` with 400 (bad input; lists supported services/components),
404 (unknown path) or 502 (status page unreachable).

## Providers and SLA sources
| Provider | SLA target | Source |
|---|---|---|
| GitHub | 99.9 % (quarterly) | https://github.com/customer-terms/github-online-services-sla |
| Cloudflare | 100 % | https://www.cloudflare.com/business-sla/ |
| Vercel | 99.99 % (monthly, Enterprise) | https://vercel.com/legal/sla |
| Twilio | 99.95 % (monthly, APIs) | https://www.twilio.com/en-us/legal/service-level-agreement/twilio-apis |
| DigitalOcean | 99.99 % (monthly, Droplets) | https://www.digitalocean.com/sla/cpu-droplets |
| Discord | 99.9 % *reference only* (no published SLA, `sla_published: false`) | none |

## Telegraph
`miner.yaml` registers `/sla` for `SLA_COMPLIANCE_GENERAL` with an `intent_mapping` for all five
response fields. Check it with `scripts/check-miner-yaml.sh`. Testnet only (Ethereum Sepolia).
