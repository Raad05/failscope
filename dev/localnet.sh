#!/usr/bin/env bash
# Local validator with the Yellowstone gRPC plugin, for developing ingest.
#
#   dev/localnet.sh            # fresh ledger
#   KEEP_LEDGER=1 dev/localnet.sh
#   GEYSER_CONFIG=dev/yellowstone-config-short-replay.json dev/localnet.sh  # 20-slot replay window
#
# RPC http://127.0.0.1:8899, Yellowstone gRPC http://127.0.0.1:10000.
# Loads fail_target/fail_callee from onchain/target/deploy (build them first
# with `anchor build --arch v0`), and clones the Program Metadata program and
# both programs' IDL accounts from devnet so IDL fetching works locally too.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
plugin="$root/.local/yellowstone-grpc/target/release/libyellowstone_grpc_geyser.so"
deploy="$root/onchain/target/deploy"

[ -f "$plugin" ] || { echo "missing $plugin: see dev/README.md" >&2; exit 1; }
for p in fail_target fail_callee; do
  [ -f "$deploy/$p.so" ] || { echo "missing $deploy/$p.so: run anchor build --arch v0 in onchain/" >&2; exit 1; }
done

reset=--reset
[ "${KEEP_LEDGER:-}" = 1 ] && reset=

exec solana-test-validator $reset \
  --ledger "$root/.local/test-ledger" \
  --geyser-plugin-config "${GEYSER_CONFIG:-$root/dev/yellowstone-config.json}" \
  --bpf-program 6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey "$deploy/fail_target.so" \
  --bpf-program BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb "$deploy/fail_callee.so" \
  --url devnet \
  --clone-upgradeable-program ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S \
  --clone HZbAvBW8W7gtrP7f5a9ucr3SirmfLoZWSRVVr5sseBE2 \
  --clone 2XgYu8FKXbVsutSHECbiXbmPRnonwVHmj8LsBsdqHgWV
