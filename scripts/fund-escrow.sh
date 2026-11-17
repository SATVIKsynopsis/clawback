#!/usr/bin/env bash
# Approve + deposit Circle test USDC into Telegraph escrow on the Diamond.
#   scripts/fund-escrow.sh 2000000   # 2.00 USDC (6 decimals)
# ON-CHAIN: sends two Sepolia transactions.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; source "$ROOT/.env"
AMT="${1:?amount in uUSDC, e.g. 2000000}"
cast send "$USDC" "approve(address,uint256)" "$DIAMOND" "$AMT" --rpc-url "$RPC" --private-key "$PRIVATE_KEY"
cast send "$DIAMOND" "depositUSDC(uint256)" "$AMT" --rpc-url "$RPC" --private-key "$PRIVATE_KEY"
curl -s "$NODE/v1/ledger/account/$ADDR" | jq '{deposited, balance, available}'
