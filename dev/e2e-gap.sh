#!/usr/bin/env bash
# End-to-end gap check: ingest is down longer than the server's replay
# window, so failures sent meanwhile can no longer be replayed. Ingest must
# say so (a recorded gap covering every lost failure), not silently skip them.
#
#   dev/e2e-gap.sh     # needs Postgres; (re)starts the local validator itself
#                      # with a 20-slot replay window
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
db=failscope_e2e_gap
export DATABASE_URL="postgres://failscope:failscope@localhost:5432/$db"
export RPC_URL=http://127.0.0.1:8899
export YELLOWSTONE_ENDPOINT=http://127.0.0.1:10000
export RUST_LOG=info
log_dir="$root/.local/e2e-gap"
mkdir -p "$log_dir"

psql_() { docker compose exec -T db psql -U failscope -d "${2:-$db}" -tA -c "$1"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ingest_pid=
trap '[ -n "$ingest_pid" ] && kill -TERM "$ingest_pid" 2>/dev/null; true' EXIT

wait_validator() {
  # Programs loaded at genesis become callable a few slots in.
  for _ in $(seq 1 60); do
    [ "$(solana -u localhost slot 2>/dev/null || echo 0)" -ge 5 ] && return
    sleep 1
  done
  fail "validator did not come up"
}
start_validator() {
  pkill -f '^solana-test-validator' 2>/dev/null || true
  for _ in $(seq 1 60); do
    pgrep -f '^solana-test-validator' >/dev/null || ss -ltn | grep -qE ':(8899|9900|10000) ' || break
    sleep 1
  done
  GEYSER_CONFIG="$root/dev/yellowstone-config-short-replay.json" \
    nohup dev/localnet.sh > "$log_dir/validator.log" 2>&1 &
  wait_validator
}
start_ingest() {
  target/debug/failscope ingest > "$log_dir/ingest-$1.log" 2>&1 &
  ingest_pid=$!
  for _ in $(seq 1 60); do grep -q subscribing "$log_dir/ingest-$1.log" && return; sleep 0.5; done
  fail "ingest did not subscribe (see $log_dir/ingest-$1.log)"
}
send_round() {
  (cd onchain && RPC_URL=$RPC_URL target/debug/send_failures) | awk '/InstructionError/ {print $2}' > "$log_dir/$1.txt"
  wc -l < "$log_dir/$1.txt"
}
wait_rows() {
  for _ in $(seq 1 60); do
    [ "$(psql_ 'SELECT count(*) FROM failed_tx')" -ge "$1" ] && return
    sleep 1
  done
  fail "expected $1 rows, have $(psql_ 'SELECT count(*) FROM failed_tx')"
}

cargo build -q -p failscope-api
(cd onchain && cargo build -q -p fail_client --bin send_failures)
psql_ "DROP DATABASE IF EXISTS $db WITH (FORCE)" failscope >/dev/null
psql_ "CREATE DATABASE $db" failscope >/dev/null

echo "fresh validator, 20-slot replay window"
start_validator
echo "round 1: live"
start_ingest 1
echo "  sent $(send_round round1)"
wait_rows 8

echo "round 2: ingest stopped, failures sent, then left to fall out of the replay window"
kill -TERM "$ingest_pid"; wait "$ingest_pid" 2>/dev/null || true
cursor=$(psql_ 'SELECT slot FROM ingest_cursor')
echo "  cursor $cursor, sent $(send_round round2)"

target=$(( $(solana -u localhost slot) + 100 ))
while [ "$(solana -u localhost slot)" -lt "$target" ]; do sleep 2; done
echo "  waited until slot $target"
start_ingest 2
echo "round 3: live again"
echo "  sent $(send_round round3)"
wait_rows 16
kill -TERM "$ingest_pid"; wait "$ingest_pid" 2>/dev/null || true

{ grep -E 'from_slot refused|gap' "$log_dir/ingest-2.log" || true; } | sed 's/^.*pipeline: /  log: /' | cut -c1-150
echo "  gaps: $(psql_ "SELECT string_agg(from_slot || '..' || to_slot || ' (' || reason || ')', ', ') FROM ingest_gaps")"

stored=$(psql_ "SELECT signature FROM failed_tx" | sort)
lost=0
for sig in $(cat "$log_dir/round2.txt"); do
  if grep -qx "$sig" <<<"$stored"; then fail "round-2 failure $sig was stored; expected it lost"; fi
  slot=$(solana -u localhost confirm -v "$sig" 2>/dev/null | sed -n 's/^Transaction executed in slot \([0-9]*\):.*/\1/p')
  [ -n "$slot" ] || fail "no slot for $sig"
  covered=$(psql_ "SELECT count(*) FROM ingest_gaps WHERE $slot BETWEEN from_slot AND to_slot")
  [ "$covered" -ge 1 ] || fail "lost failure $sig at slot $slot is not inside a recorded gap"
  lost=$((lost + 1))
done
for sig in $(cat "$log_dir/round1.txt" "$log_dir/round3.txt"); do
  grep -qx "$sig" <<<"$stored" || fail "$sig missing"
done
echo "PASS: rounds 1 and 3 stored (16); $lost round-2 failures fell out of the replay window and every one lies inside a recorded gap"
