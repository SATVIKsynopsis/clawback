# After: sla-miner live on Telegraph (2026-11-17)

Miner id `7410311` (slug `sla-miner`, registration `5310`), `activation_status: active` at 17:24:50Z.
Same request as before: `{"intent":"SLA_COMPLIANCE_GENERAL","params":{"service":"GitHub","period":"last 30 days"}}`.

| Call | Miner | Answer | field_status | Receipt | Signal |
|---|---|---|---|---|---|
| Direct `/engine/v1/ask/7410311` | **Statuspage SLA Auditor** | GitHub **99.2053%** vs SLA **99.9%** → `compliant: false`, period "last 30 days" (1.1 s) | **all ok** | `0xf2e071fb9a7a9ca8662d71ca7a567e7703c2f54ff409dd8f7eb55564d6c4f0c3` | `0x5e64e1df22b1c8a71d1d737db33dd4f6ac2f424be1a0463d9fce0fa016c6cb8c` |
| Routed `/engine/v1/ask` | GitHub Components SLA (103010) | service only; uptime/target/period **missing**, compliant **rejected** (charged $0.01) | 1 of 5 ok | `0xbca07e495f2c03410e36ecb5312c5d48fc830c40f384dfe5ea3efd506d49201c` | `0xa2ce0642c351f1d5f9d3896a1cc5bbd57cecae0bbaef985a48e5cfdd26923b06` |

Routed traffic still reaches the incumbent: new miners share a 5% routing slice during their 7-day grace
period. This is why Clawback validates every routed answer and falls back to a direct ask.
The 99.2053% figure matches the hand verification in `sla-miner/VERIFICATION.md`.
