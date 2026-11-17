# Paid end-to-end Clawback demo (2026-11-17, testnet)

`docker compose up` with a funded wallet, then `curl -X POST localhost:8091/fail` at 17:28:13Z.

| Time (UTC) | Step |
|---|---|
| 17:28:56 | breach: observed uptime 16.67% over 120 s < GitHub SLA 99.9%, incident #1 opened |
| 17:29:01 | routed SLA_COMPLIANCE_GENERAL answer from GitHub Components SLA **rejected** (uptime/target/compliant missing), paid $0.01, receipt `0x883c40590151d2677b6fc30f3e65b83b8fde6b2405106d9618e89f8a399610b8` |
| 17:29:05 | direct-ask fallback to sla-miner (7410311) **validated**: GitHub 99.2053% vs 99.9%, receipt `0x8e8f47ead1cbf5207c0b6039b725c4dbb8e9c09f59a2e631b6a5cd046c20320f`, signal `0xb5f58066167ebbf83099cfe3dc30da3e0f2cdfd5c73542b2f46f2ec06ea30e36` |
| 17:29:05 | claim drafted: **$210.00** (GitHub tier 99.0–99.5% → 10% of $2,100 configured fees) |

Spend: $0.02 for 2 answers (1 rejected). Credit claimed / intelligence spent = 10,500×.
The $2,100 fee base and the account are demo config values (`app/clawback.toml`), not a real customer.

Files: `claim-incident-1.eml` (the drafted email with `telegraph-evidence.json` attached),
`dashboard-state.json` (`/api/state`), `dashboard.png`.
