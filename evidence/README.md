# Evidence

## Before (Oct 31, old Diamond): GitHub question routed to Cloudflare, 2 of 2 paid calls wrong
Both paid routed asks for `{"service":"GitHub","period":"last 30 days"}` went to **Cloudflare Status SLA (103003)**,
which returned Cloudflare's *current* status (`result_shape: raw`, `shape_error: no_fields_found`) and charged $0.01.

| Call | Receipt hash | Signal hash |
|---|---|---|
| 1 (primary, raw files here) | `0x9a853e3f1f35cce953ba0f9d55835f417f98a3af5d9f92198e99be7a091d4698` | `0x5f42b81de2751c943948f9c9c117ff5d6b4598f2ea8f34486dc5f3022d2fcd52` |
| 2 | `0x34e6ec323180642ddee4472a789f7692977a2825ed1da33d0e618e0c7b952e1d` | `0x7ea8c8fc752901f2a372fd3fa6ab950defaddf40009456580d8f9b915f7e5231` |

Files: `before-github-sla.txt`, `before-github-sla.response.json`, `before-github-sla.headers.txt`.
Note: as of 2026-11-17, `GET /engine/v1/signal/{hash}` returns `signal not found` for both (the Diamond migrated
on Nov 17), so the saved responses and receipts are the record.

## Comparison (Nov 17, new Diamond)
See [comparison/](comparison/): the #1-ranked miner was paid for an answer with no uptime, and the zk miner
declined the request.

## After
_Pending miner registration: a paid direct ask to sla-miner and a routed ask._
