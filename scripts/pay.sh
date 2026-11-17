#!/usr/bin/env bash
# Paid Telegraph ask via the escrow x402 flow.
#
#   scripts/pay.sh [BODY_JSON] [MINER_ID_OR_SLUG]
#
#   BODY_JSON         request body (default: a CRYPTO_PRICE_USD smoke test)
#   MINER_ID_OR_SLUG  optional; if given, POST /engine/v1/ask/{miner} (direct ask, no fallback)
#
# Needs .env at the repo root with PRIVATE_KEY, ADDR, DIAMOND, NODE.
# Outputs go to .pay/ (gitignored): challenge.json, headers.txt, response.json, receipt.json.
# The private key is only passed to `cast wallet sign`; it is never printed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/.env"

DEFAULT='{"intent":"CRYPTO_PRICE_USD","params":{"symbol":"BTC"}}'
BODY="${1:-$DEFAULT}"
MINER="${2:-}"
OUT="$ROOT/.pay"
mkdir -p "$OUT"

echo "$BODY" | jq . >/dev/null || { echo "BODY is not valid JSON: $BODY" >&2; exit 1; }

if [[ -n "$MINER" ]]; then
  URL="$NODE/engine/v1/ask/$MINER"
else
  URL="$NODE/engine/v1/ask"
fi
echo "POST $URL"

# 1. Request a fresh payment challenge (expect 402)
CODE=$(curl -sS -o "$OUT/challenge.json" -w '%{http_code}' -X POST "$URL" \
  -H 'Content-Type: application/json' -d "$BODY")
if [[ "$CODE" != "402" ]]; then
  echo "Expected 402 challenge, got $CODE (nothing was paid):" >&2
  jq . "$OUT/challenge.json" 2>/dev/null || cat "$OUT/challenge.json"
  exit 1
fi

# 2. Extract escrow payment details
TEMPLATE=$(jq -r '.accepts[] | select(.scheme=="escrow") | .extra.message' "$OUT/challenge.json")
PAYTO=$(jq -r '.accepts[] | select(.scheme=="escrow") | .payTo' "$OUT/challenge.json")
NETWORK=$(jq -r '.accepts[] | select(.scheme=="escrow") | .network' "$OUT/challenge.json")
VALUE=$(printf '%s\n' "$TEMPLATE" | sed -n 's/^Amount:[[:space:]]*\([0-9]*\) uUSDC$/\1/p')

if [[ -z "$TEMPLATE" || -z "$VALUE" ]]; then
  echo "Could not extract payment challenge." >&2
  jq . "$OUT/challenge.json"
  exit 1
fi

# 3. Validate recipient, network and amount before signing
LADDR=$(printf '%s' "$ADDR" | tr '[:upper:]' '[:lower:]')
PAYTO=$(printf '%s' "$PAYTO" | tr '[:upper:]' '[:lower:]')
EXPECTED_TO=$(printf '%s' "$DIAMOND" | tr '[:upper:]' '[:lower:]')
MAX_UUSDC="${MAX_UUSDC:-50000}"   # refuse anything above $0.05 unless overridden

if [[ "$PAYTO" != "$EXPECTED_TO" || "$NETWORK" != "eip155:11155111" ]]; then
  echo "Unexpected payment recipient or network. Aborting." >&2
  exit 1
fi
if (( VALUE > MAX_UUSDC )); then
  echo "Price $VALUE uUSDC exceeds MAX_UUSDC=$MAX_UUSDC. Aborting." >&2
  exit 1
fi
echo "Payment amount: $VALUE uUSDC -> $PAYTO"

# 4. Authorization fields (window <= 2 min, fresh nonce every attempt)
NOW=$(date +%s)
VA=$((NOW - 5))
VB=$((NOW + 100))
NONCE="cb-$NOW-$RANDOM$RANDOM"

MESSAGE=$(printf '%s' "$TEMPLATE" | sed \
  -e "s/{from}/$LADDR/g" -e "s/{nonce}/$NONCE/g" \
  -e "s/{validAfter}/$VA/g" -e "s/{validBefore}/$VB/g")

# 5. EIP-191 personal_sign
SIG=$(cast wallet sign --private-key "$PRIVATE_KEY" "$MESSAGE")

# 6. X-PAYMENT header
PAYMENT=$(jq -cn --arg s "$SIG" --arg f "$LADDR" --arg t "$PAYTO" --arg v "$VALUE" \
  --arg a "$VA" --arg b "$VB" --arg n "$NONCE" \
  '{x402Version:2, scheme:"escrow", network:"eip155:11155111",
    payload:{signature:$s, authorization:{from:$f,to:$t,value:$v,validAfter:$a,validBefore:$b,nonce:$n}}}' \
  | base64 | tr -d '\n')

# 7. Paid request
curl -sS -D "$OUT/headers.txt" -o "$OUT/response.json" -X POST "$URL" \
  -H 'Content-Type: application/json' -H "X-PAYMENT: $PAYMENT" -d "$BODY"

echo ""
echo "===== RESPONSE ====="
jq . "$OUT/response.json" || cat "$OUT/response.json"

# 8. Receipt
RECEIPT=$(grep -i '^x-payment-response:' "$OUT/headers.txt" | awk '{print $2}' | tr -d '\r' || true)
if [[ -n "$RECEIPT" ]]; then
  echo ""
  echo "===== PAYMENT RECEIPT ====="
  printf '%s' "$RECEIPT" | base64 -D | jq . | tee "$OUT/receipt.json"
else
  echo "No X-PAYMENT-RESPONSE header (not billed)."
  rm -f "$OUT/receipt.json"
fi
