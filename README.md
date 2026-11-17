# Clawback

Telegraph Hackathon Season II entry (testnet). Two parts in one repo:

- **[`sla-miner/`](sla-miner/)**: Telegraph miner for `SLA_COMPLIANCE_GENERAL`. Historical uptime
  from providers' public status-page incidents vs their published SLA.
- **[`app/`](app/)**: Clawback, an agent that detects SLA breaches, buys independent proof
  through Telegraph, validates it, and drafts the SLA credit claim. `docker compose up` runs the demo.

Also: [`evidence/`](evidence/) (receipts, comparisons) and [`scripts/`](scripts/) (`pay.sh` paid ask,
`balance.sh`, `fund-escrow.sh`, `check-miner-yaml.sh`). Copy `env.example` to `.env` to use the scripts.
