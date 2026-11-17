#!/usr/bin/env bash
# Read-only: wallet USDC + Telegraph escrow ledger balance.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; source "$ROOT/.env"
echo "wallet USDC (uUSDC): $(cast call "$USDC" 'balanceOf(address)(uint256)' "$ADDR" --rpc-url "$RPC")"
curl -s "$NODE/v1/ledger/account/$ADDR" | jq '{deposited, balance, available, unbilled}'
