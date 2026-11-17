# Comparison: "GitHub, last 30 days" on Telegraph (2026-11-17)

Same request both times: `{"intent":"SLA_COMPLIANCE_GENERAL","params":{"service":"GitHub","period":"last 30 days"}}`,
paid from escrow via `scripts/pay.sh` (Ethereum Sepolia testnet, new Diamond `0x72a7…a08b`).

| Call | Miner | HTTP | Billed | Answer |
|---|---|---|---|---|
| Direct ask `/engine/v1/ask/781347` | zk-sla-http | 500 | no (no receipt) | upstream 422 `unsupported_measurement`, verdict "Insufficient information" |
| Routed ask `/engine/v1/ask` | **GitHub Components SLA** (103010, ranked #1 for the intent) | 200 | **yes, 10000 uUSDC** | `result_shape: intent`, but `uptime_pct` missing, `sla_target_pct` missing, `compliant` rejected |

Routed receipt: `0x8a0d3fbb930b54696082ecb4b653f580b8763ec5424e61573acfa9917f3ba608` (epoch 3)
Routed signal hash: `0x046044a86bc7eadd46c55e42410ba56feb79619e507450e502c651f4598ba428`

Clawback's validator rejects this answer. The same JSON is pinned as a unit test
(`proof::tests::rejects_real_routed_answer_2026_11_17`).

## Why the zk-* miners can't answer this
The zk-* family reports homepage reachability from Upptime/HTTP probes for a fixed set of URLs. It does not
measure the provider's own declared incidents against the provider's published SLA target, and it has no
`compliant` verdict for an SLA. sla-miner computes uptime from the provider's status-page incident history
(component outage time) and compares it to the published SLA (cited per provider), returning `compliant`.

## Scoring observation
Epoch-0 scores for SLA_COMPLIANCE_GENERAL ranked current-status wrappers highest (sla-gh-comp 0.9767 #1,
sla-github-sum 0.9767 #2). This routed call shows the #1 miner returning no uptime for a historical
question and still being paid. Clawback therefore validates every answer and falls back to a direct ask.

Files: `*.response.json` (body), `*.headers.txt` (incl. `X-PAYMENT-RESPONSE`), `routed.receipt.json`.
