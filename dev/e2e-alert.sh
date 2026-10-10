#!/usr/bin/env bash
# End-to-end alert check against the local validator (dev/localnet.sh) and
# Postgres. Short windows so it finishes in ~10 minutes:
#   window 1 min, baseline 6 min, factor 3, min 5 failures, tick 10s.
#
# 1. ~6 min of quiet traffic (one failure every 20s) builds a baseline.
# 2. A burst (3 rounds of all 8 cases) must fire an alert for fail_target.
# 3. Back to quiet: the alert must resolve.
# Checks the webhook receiver saw exactly firing then resolved for fail_target.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
db=failscope_e2e_alert
target=6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey
export DATABASE_URL="postgres://failscope:failscope@localhost:5432/$db"
export RPC_URL=http://127.0.0.1:8899
export YELLOWSTONE_ENDPOINT=http://127.0.0.1:10000
export RUST_LOG=info
log_dir="$root/.local/e2e-alert"
mkdir -p "$log_dir"
hooks="$log_dir/webhooks.jsonl"
: > "$hooks"

psql_() { docker compose exec -T db psql -U failscope -d "${2:-$db}" -tA -c "$1"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
pids=()
trap 'for p in "${pids[@]}"; do kill -TERM "$p" 2>/dev/null; done; true' EXIT

solana -u localhost slot >/dev/null 2>&1 || fail "local validator not running (dev/localnet.sh)"
cargo build -q -p failscope-api
(cd onchain && cargo build -q -p fail_client --bin send_failures)
send() { (cd onchain && RPC_URL=$RPC_URL target/debug/send_failures "$@") > /dev/null; }
psql_ "DROP DATABASE IF EXISTS $db WITH (FORCE)" failscope >/dev/null
psql_ "CREATE DATABASE $db" failscope >/dev/null

python3 dev/webhook_sink.py 9911 "$hooks" & pids+=($!)
target/debug/failscope ingest > "$log_dir/ingest.log" 2>&1 & pids+=($!)
ALERT_WEBHOOK_URL=http://127.0.0.1:9911/hook target/debug/failscope alert \
  --window-minutes 1 --baseline-minutes 6 --factor 3 --min-failures 5 --interval-secs 10 \
  --dashboard-url http://127.0.0.1:8080 > "$log_dir/alert.log" 2>&1 & pids+=($!)

cases=(custom_error has_one_violation missing_signer overflow_panic overflow_checked compute_exhausted)
echo "phase 1: quiet baseline (6 min)"
end=$(( $(date +%s) + 360 ))
while [ "$(date +%s)" -lt "$end" ]; do
  send "${cases[RANDOM % ${#cases[@]}]}"
  sleep 20
done
[ ! -s "$hooks" ] || fail "alerted during the quiet phase: $(cat "$hooks")"
echo "  baseline: $(psql_ "SELECT count(*) FROM failed_tx WHERE failing_program_id = '$target'") fail_target failures, no alert"

echo "phase 2: burst"
for _ in 1 2 3; do send; done
for _ in $(seq 1 18); do grep -q '"firing"' "$hooks" && break; sleep 5; done
grep -q '"firing"' "$hooks" || fail "no firing alert within 90s of the burst"
python3 -c 'import json,sys; e=json.loads(open(sys.argv[1]).readline()); print("  fired:", e["text"])' "$hooks"

echo "phase 3: quiet again"
for _ in $(seq 1 36); do grep -q '"resolved"' "$hooks" && break; sleep 5; done
grep -q '"resolved"' "$hooks" || fail "alert did not resolve within 3 min"

python3 - "$hooks" "$target" <<'PY'
import json, sys
events = [json.loads(l) for l in open(sys.argv[1])]
seq = [(e["program_id"], e["state"]) for e in events]
target = sys.argv[2]
assert seq == [(target, "firing"), (target, "resolved")], seq
print("  webhooks:", " -> ".join(s for _, s in seq))
PY
echo "  recorded: $(psql_ "SELECT string_agg(state, ' -> ' ORDER BY id) FROM alert_events")"
echo "PASS: quiet baseline stayed silent, the burst fired once, and it resolved"
