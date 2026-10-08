# failscope

> Streams Solana transactions that **failed on-chain**, decodes *why* they failed (including custom Anchor errors inside CPIs, attributed to the program that actually failed), stores them, and serves an API and dashboard.

_Screenshot: coming in M7._

**Status:** M0 (project setup). See [PLAN.md](PLAN.md) for the roadmap.

## Failed vs dropped

failscope only sees transactions that **landed in a block** and then failed during execution (the fee was charged, the state changes were rolled back). Transactions that were **dropped** (never landed: expired blockhash, rejected in preflight, lost in forwarding) never reach Geyser, so they are invisible here. This is not a landing-rate tool.

## Architecture

_Diagram coming with M5._

```
Yellowstone gRPC ──> ingest ──> decoder ──> store (Postgres) ──> api + dashboard
                                   │
                                   └── idl (on-chain Anchor IDL fetch + cache)
```

| Path | What it is |
|---|---|
| `crates/decoder` | Pure decoding: logs + error + account keys → attributed, classified failure. No I/O. |
| `crates/idl` | Fetches and caches Anchor IDLs from chain. |
| `crates/ingest` | Yellowstone gRPC subscription, reconnect/resume. |
| `crates/store` | `Store` trait + Postgres implementation. |
| `crates/api` | `failscope` binary: HTTP API and dashboard. |
| `onchain/` | Separate Anchor workspace: `fail_target`, a program that fails on purpose in distinct ways. |
| `fixtures/` | Recorded failed transactions with expected decode results. |
| `docs/decisions.md` | Design decisions and verified spec details. |

## Decoding pipeline

_Described in M3._

## Companion program failure modes

_Table filled in M1/M3._

| # | Failure | How it fails | Decoded as |
|---|---|---|---|

## Decoder coverage

_Numbers from a real run, after M5._

## Development

Requirements: Rust (toolchains are pinned per workspace by `rust-toolchain.toml`), and for `onchain/` the Solana CLI and Anchor CLI 1.2.

```sh
cp .env.example .env

# off-chain workspace
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run          # starts the failscope binary

# on-chain workspace
cd onchain
cargo test
anchor build
```

## What I learned

_Written as the project goes._
