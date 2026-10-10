#!/usr/bin/env bash
# Sends a random, uneven mix of the companion program's failure cases to the
# local validator, so ingest and the dashboard have something time-shaped to
# show. Synthetic: every failure comes from fail_target / fail_callee.
#
#   dev/traffic.sh [minutes]   # default 10
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
minutes=${1:-10}
end=$(( $(date +%s) + minutes * 60 ))
# Weighted: common failures appear more often.
cases=(custom_error custom_error custom_error custom_error
       overflow_checked overflow_checked overflow_checked
       has_one_violation has_one_violation
       nested_cpi_callee nested_cpi_callee
       compute_exhausted cpi_system_transfer cpi_token_transfer missing_signer
       overflow_panic)
bin="$root/onchain/target/debug/send_failures"
(cd "$root/onchain" && cargo build -q -p fail_client --bin send_failures)
sent=0
while [ "$(date +%s)" -lt "$end" ]; do
  # Bursts now and then, quiet stretches otherwise.
  n=$(( RANDOM % 10 < 2 ? 4 + RANDOM % 4 : 1 + RANDOM % 2 ))
  picks=()
  for _ in $(seq 1 "$n"); do picks+=("${cases[RANDOM % ${#cases[@]}]}"); done
  RPC_URL=http://127.0.0.1:8899 "$bin" "${picks[@]}" > /dev/null
  # send_failures sends each named case once, so count distinct picks.
  sent=$(( sent + $(printf '%s\n' "${picks[@]}" | sort -u | wc -l) ))
  sleep $(( 2 + RANDOM % 12 ))
done
echo "sent $sent failures over $minutes minutes"
