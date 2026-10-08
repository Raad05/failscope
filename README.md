# failscope

> Streams Solana transactions that **failed on-chain**, decodes *why* they failed (including custom Anchor errors inside CPIs, attributed to the program that actually failed), stores them, and serves an API and dashboard.

_Screenshot: coming in M7._

**Status:** M1 done (companion program on devnet). See [PLAN.md](PLAN.md) for the roadmap.

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

`onchain/` holds two Anchor programs deployed on devnet: `fail_target` (one instruction per failure mode) and `fail_callee` (fails inside a nested CPI). Signatures and explorer links are in [fixtures/README.md](fixtures/README.md).

| # | Failure | Transaction error | Program that actually failed | Decoded as |
|---|---|---|---|---|
| 1 | `require!` user error | `InstructionError(0, Custom(6000))` | fail_target | _M3_ |
| 2 | `has_one` constraint | `InstructionError(0, Custom(2001))` | fail_target | _M3_ |
| 3 | missing signer | `InstructionError(0, Custom(3010))` | fail_target | _M3_ |
| 4a | unchecked overflow (panic) | `InstructionError(0, ProgramFailedToComplete)` | fail_target | _M3_ |
| 4b | checked overflow | `InstructionError(0, Custom(6001))` | fail_target | _M3_ |
| 5 | compute exhaustion | `InstructionError(1, ProgramFailedToComplete)` | fail_target | _M3_ |
| 6 | CPI to System with bad transfer | `InstructionError(0, Custom(1))` | System program | _M3_ |
| 7 | nested CPI, callee error | `InstructionError(0, Custom(6000))` | fail_callee | _M3_ |

Rows 1 and 7 share the exact same transaction error, and rows 4a and 5 share the same variant. Telling them apart is the decoder's job.

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
anchor build --arch v0   # SBPF v0; tests load target/deploy/*.so
cargo test               # LiteSVM: runs every failure case locally
cargo run -p fail_client --bin send_failures   # devnet only; writes fixtures/signatures.json
```

## What I learned

_Written as the project goes._
