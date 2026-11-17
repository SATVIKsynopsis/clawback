#!/usr/bin/env bash
# Approve + deposit test USDC into Telegraph escrow on the Diamond.
# The token is read from the Diamond (usdcToken()); on 2026-11-17 that was the old
# mintable test USDC 0xcc19...Ae17, not Circle's USDC as the docs say.
#   scripts/fund-escrow.sh 2000000   # 2.00 USDC (6 decimals)
# ON-CHAIN: sends two Sepolia transactions.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; source "$ROOT/.env"
AMT="${1:?amount in uUSDC, e.g. 2000000}"
TOKEN=$(cast call "$DIAMOND" 'usdcToken()(address)' --rpc-url "$RPC")
echo "escrow token: $TOKEN"
cast send "$TOKEN" "approve(address,uint256)" "$DIAMOND" "$AMT" --rpc-url "$RPC" --private-key "$PRIVATE_KEY"
# load-balanced RPCs can lag: wait until the allowance is visible before depositing
for _ in $(seq 20); do
  A=$(cast call "$TOKEN" 'allowance(address,address)(uint256)' "$ADDR" "$DIAMOND" --rpc-url "$RPC" | awk '{print $1}')
  [[ "$A" -ge "$AMT" ]] && break; sleep 3
done
cast call "$DIAMOND" "depositUSDC(uint256)" "$AMT" --from "$ADDR" --rpc-url "$RPC" >/dev/null   # simulate first
cast send "$DIAMOND" "depositUSDC(uint256)" "$AMT" --rpc-url "$RPC" --private-key "$PRIVATE_KEY"
curl -s "$NODE/v1/ledger/account/$ADDR" | jq '{deposited, balance, available}'
