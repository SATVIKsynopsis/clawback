#!/usr/bin/env bash
# Read-only: wallet USDC + Telegraph escrow ledger balance.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; source "$ROOT/.env"
TOKEN=$(cast call "$DIAMOND" 'usdcToken()(address)' --rpc-url "$RPC")
echo "escrow token: $TOKEN"
echo "wallet balance of it (uUSDC): $(cast call "$TOKEN" 'balanceOf(address)(uint256)' "$ADDR" --rpc-url "$RPC")"
curl -s "$NODE/v1/ledger/account/$ADDR" | jq '{deposited, balance, available, unbilled}'
