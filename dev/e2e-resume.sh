#!/usr/bin/env bash
# End-to-end resume check against the local validator (dev/localnet.sh) and
# Postgres (docker compose up -d db). Uses a throwaway database.
#
#   dev/e2e-resume.sh
#
# Round 1: ingest runs, 8 failures are sent, all 8 must be stored.
# Round 2: ingest is SIGKILLed, 8 failures are sent while it is down, ingest
#          restarts and must replay them from the cursor.
# Round 3: same, but with a graceful SIGTERM.
# Passes if every sent signature is stored exactly once and no gap is recorded.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
db=failscope_e2e
export DATABASE_URL="postgres://failscope:failscope@localhost:5432/$db"
export RPC_URL=http://127.0.0.1:8899
export YELLOWSTONE_ENDPOINT=http://127.0.0.1:10000
export RUST_LOG=info
log_dir="$root/.local/e2e"
mkdir -p "$log_dir"
: > "$log_dir/sent.txt"

psql_() { docker compose exec -T db psql -U failscope -d "${2:-$db}" -tA -c "$1"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ingest_pid=
trap '[ -n "$ingest_pid" ] && kill -TERM "$ingest_pid" 2>/dev/null; true' EXIT

solana -u localhost slot >/dev/null 2>&1 || fail "local validator not running (dev/localnet.sh)"
psql_ "DROP DATABASE IF EXISTS $db WITH (FORCE)" failscope >/dev/null
psql_ "CREATE DATABASE $db" failscope >/dev/null
cargo build -q -p failscope-api
(cd onchain && cargo build -q -p fail_client --bin send_failures)

start_ingest() {
  target/debug/failscope ingest > "$log_dir/ingest-$1.log" 2>&1 &
  ingest_pid=$!
  for _ in $(seq 1 60); do grep -q subscribing "$log_dir/ingest-$1.log" && return; sleep 0.5; done
  fail "ingest did not subscribe (see $log_dir/ingest-$1.log)"
}

send_round() {
  (cd onchain && RPC_URL=$RPC_URL target/debug/send_failures) \
    | awk '/InstructionError/ {print $2}' | tee -a "$log_dir/sent.txt" | wc -l
}

wait_rows() {
  for _ in $(seq 1 60); do
    [ "$(psql_ 'SELECT count(*) FROM failed_tx')" -ge "$1" ] && return
    sleep 1
  done
  fail "expected $1 rows, have $(psql_ 'SELECT count(*) FROM failed_tx')"
}

check() {
  local want=$1 rows distinct gaps missing
  rows=$(psql_ "SELECT count(*) FROM failed_tx")
  distinct=$(psql_ "SELECT count(DISTINCT signature) FROM failed_tx")
  gaps=$(psql_ "SELECT count(*) FROM ingest_gaps")
  missing=$(comm -23 <(sort "$log_dir/sent.txt") <(psql_ "SELECT signature FROM failed_tx" | sort) | wc -l)
  echo "  rows=$rows distinct=$distinct sent=$(wc -l < "$log_dir/sent.txt") missing=$missing gaps=$gaps"
  [ "$rows" -eq "$want" ] && [ "$distinct" -eq "$want" ] && [ "$missing" -eq 0 ] && [ "$gaps" -eq 0 ] \
    || fail "round check failed"
}

echo "round 1: live ingest"
start_ingest 1
echo "  sent $(send_round)"
wait_rows 8
check 8

echo "round 2: SIGKILL, send while down, restart"
kill -9 "$ingest_pid"; wait "$ingest_pid" 2>/dev/null || true
echo "  cursor at kill: $(psql_ 'SELECT slot FROM ingest_cursor')"
echo "  sent $(send_round)"
start_ingest 2
{ grep -o 'from_slot=Some([0-9]*)' "$log_dir/ingest-2.log" || echo 'from_slot=none'; } | head -1 | sed 's/^/  resumed with /'
wait_rows 16
check 16

echo "round 3: SIGTERM, send while down, restart"
kill -TERM "$ingest_pid"; wait "$ingest_pid" 2>/dev/null || true
grep -q 'consumer drained' "$log_dir/ingest-2.log" || fail "no graceful drain on SIGTERM"
echo "  cursor at stop: $(psql_ 'SELECT slot FROM ingest_cursor')"
echo "  sent $(send_round)"
start_ingest 3
{ grep -o 'from_slot=Some([0-9]*)' "$log_dir/ingest-3.log" || echo 'from_slot=none'; } | head -1 | sed 's/^/  resumed with /'
wait_rows 24
check 24
kill -TERM "$ingest_pid"; wait "$ingest_pid" 2>/dev/null || true

echo "PASS: 24 failures sent across 2 restarts, 24 stored once each, no gaps"
