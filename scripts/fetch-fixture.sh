#!/usr/bin/env bash
# Saves a transaction as a decoder fixture: fixtures/txs/<name>/tx.json
#
#   scripts/fetch-fixture.sh <name> <signature> [rpc_url]
#
# Uses getTransaction with encoding=json (raw, not jsonParsed: the decoder's
# RPC adapter reads programIdIndex/accountKeys) and
# maxSupportedTransactionVersion=0 so v0 transactions with ALTs are returned.
set -euo pipefail

name=${1:?usage: fetch-fixture.sh <name> <signature> [rpc_url]}
sig=${2:?usage: fetch-fixture.sh <name> <signature> [rpc_url]}
rpc=${3:-${RPC_URL:-https://api.devnet.solana.com}}

dir="$(dirname "$0")/../fixtures/txs/$name"
mkdir -p "$dir"

body=$(jq -nc --arg sig "$sig" '{
  jsonrpc: "2.0", id: 1, method: "getTransaction",
  params: [$sig, {encoding: "json", commitment: "confirmed", maxSupportedTransactionVersion: 0}]
}')

resp=$(curl -sS --fail --retry 3 --retry-delay 2 -H 'content-type: application/json' -d "$body" "$rpc")

if [ "$(jq -r '.result == null' <<<"$resp")" = true ]; then
  echo "no transaction for $sig on $rpc: $(jq -c '.error // "null result"' <<<"$resp")" >&2
  exit 1
fi

jq '.result' <<<"$resp" > "$dir/tx.json"
echo "$dir/tx.json  err=$(jq -c '.meta.err' "$dir/tx.json")"
