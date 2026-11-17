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

## Registration (Nov 17, Ethereum Sepolia)
sla-miner registered from the CLI (`cast`), registration id **5309**, intent `SLA_COMPLIANCE_GENERAL`, min price 10000 uUSDC.

| Step | Tx |
|---|---|
| Bond approve (1 USDC, escrow token `0xcc19…Ae17`) | `0x40b4234b0d2390df61c27a4d82a4eab6a1e22bc6e6309b161a155ea1c5cbfba2` |
| `registerMiner` | `0x4cbff2175700c9444c5c4021c63fffb5f9418c229c0749432def114bda6f4788` |

YAML: `https://raw.githubusercontent.com/SATVIKsynopsis/clawback/main/sla-miner/miner.yaml`,
sha256 `0x82dd50ae2da79e0ae5f088d55dda92e8456b4c1caf811bbb471115afe29b4857` (exact hosted bytes).

Escrow funding: approve `0x3fe875db71e21a3498eb28292c9ef8a83e334a31007ded3da8bfdad3bbb85e5c`,
deposit `0x200d03f8e69fc97e5bd7e62a09662fba8fb4f7870b3ab47be50713265a4c0b56` (2.00 USDC).

## After
_Pending activation: a paid direct ask to sla-miner and a routed ask._
