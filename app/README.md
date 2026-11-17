# Clawback (app)

Watches the services you depend on. When one breaches its SLA it buys **independent proof** through
Telegraph (`SLA_COMPLIANCE_GENERAL`), **validates** the answer, computes the credit owed from the
provider's published credit tiers, and drafts the claim email with the Telegraph receipts attached.

> Full README (problem, architecture diagram, costs, limitations) lands in Phase E.

## Quickstart (demo, testnet)
```bash
cp env.example .env            # repo root; set PRIVATE_KEY to a Sepolia test wallet with escrow funded
docker compose up --build      # dashboard: http://localhost:8090   mock: http://localhost:8091
curl -X POST localhost:8091/fail      # within ~1 min: breach -> paid, validated proof -> claim draft
curl -X POST localhost:8091/recover
```
Without `PRIVATE_KEY` everything runs, but proofs are refused and no claim is filed.

## Commands
```bash
cargo run -p clawback -- check --config app/clawback.toml          # free: config + wallet + budget
cargo run -p clawback -- prove --config app/clawback.toml --service GitHub   # one paid, validated proof
cargo run -p clawback -- run   --config app/clawback.toml          # watcher + dashboard
cargo run -p clawback -- mock                                      # demo target on :8091
cargo test -p clawback
```

## How a proof is accepted
Routed ask first. The answer must have `result_shape == "intent"`, a `result.service` naming the
service we asked about, `uptime_pct` in 0–100, `sla_target_pct`, a `compliant` consistent with
both, and every `field_status` "ok". A rejected answer is still recorded (cost + receipt), then
**one** direct ask to `fallback_miner` is tried. No validated proof means no claim. If the proof
says the provider *met* its SLA, the incident is marked `not_confirmed` and no claim is filed.

## Spend limits
`max_price_uusdc` per answer, `per_incident_uusdc`, `per_day_uusdc`. All are checked **before signing**.
Budget 0 refuses with a reason. Env overrides: `CLAWBACK_BUDGET_PER_INCIDENT_UUSDC`,
`CLAWBACK_BUDGET_PER_DAY_UUSDC`. The wallet only ever signs for the configured Diamond `payTo`.
