# Local development stack

Everything M5 needs runs locally: a validator with the Yellowstone gRPC
plugin, and Postgres. No hosted endpoint required.

## One-time setup

1. **Postgres via Docker.** Your user needs to be in the `docker` group:
   `sudo usermod -aG docker $USER`, then log in again (or prefix docker
   commands with `sg docker -c '…'`).
2. **Build the Yellowstone plugin** against the exact Agave version of your
   `solana-test-validator` (here 3.1.14). The plugin is loaded into the
   validator process, so versions must match.
   ```sh
   git clone --depth 1 --branch 'v12.1.0+solana.3.1.10' \
     https://github.com/rpcpool/yellowstone-grpc.git .local/yellowstone-grpc
   cd .local/yellowstone-grpc
   sed -i 's/"=3\.1\.10"/"=3.1.14"/g' Cargo.toml   # match the local validator
   cargo build --release -p yellowstone-grpc-geyser  # Rust 1.86 via its rust-toolchain.toml
   ```
   `v12.1.0+solana.3.1.10` is the newest release built for Agave 3.1.x. Agave
   3.1.14 and the plugin both build with Rust 1.86.0.
3. **Build the programs**: `cd onchain && anchor build --arch v0`.

## Run

```sh
docker compose up -d db          # Postgres on 127.0.0.1:5432
dev/localnet.sh                  # validator: RPC :8899, Yellowstone gRPC :10000
cp .env.example .env
cargo run -p failscope-api -- ingest
# in another terminal: produce failures
cd onchain && RPC_URL=http://127.0.0.1:8899 cargo run -p fail_client --bin send_failures
```

`dev/localnet.sh` loads `fail_target`/`fail_callee` at genesis and clones the
Program Metadata program plus both IDL accounts from devnet, so IDL fetching
works locally. `dev/yellowstone-config.json` keeps 5000 slots for `from_slot`
replay.

## Dashboard with data

```sh
cargo run -p failscope-api -- serve   # http://127.0.0.1:8080
dev/traffic.sh 10                     # random mix of failure cases for 10 minutes
```

## End-to-end checks

- `dev/e2e-resume.sh`: ingest is SIGKILLed and later SIGTERMed while failures
  keep arriving. After each restart it must resume from the cursor: every
  sent failure is stored exactly once, and no gap is recorded.
- `dev/e2e-gap.sh`: the validator runs with a 20-slot replay window
  (`dev/yellowstone-config-short-replay.json`). Ingest stays down past it, and
  every failure sent meanwhile must fall inside a recorded gap.

- `dev/e2e-alert.sh`: alerting on a live stream with short windows (1 min window, 6 min baseline, 10s ticks). A quiet baseline must not alert, a burst must fire once, and quiet again must resolve. Webhooks go to `dev/webhook_sink.py`. Takes about 10 minutes.

All use throwaway databases (`failscope_e2e`, `failscope_e2e_gap`, `failscope_e2e_alert`).
