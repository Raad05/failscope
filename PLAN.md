# Failscope: Solana failed-transaction analyzer (Rust)

Working name: **failscope**. Rename freely.

## 1. Goal

Stream Solana transactions via Yellowstone gRPC (Geyser), keep the ones that **failed on-chain**, decode *why* they failed (including custom Anchor errors, attributed to the correct program), store them, and expose an API plus a small dashboard.

Portfolio signal: deep understanding of Solana's transaction model, error handling across CPIs, IDLs, and streaming infrastructure, plus a companion on-chain program written for testing.

### Non-goals / known limits (put these in the README)
- Geyser only sees transactions that **landed in a block**. Dropped or never-landed transactions are invisible to this tool. Failed (executed, then rejected) and dropped are different problems.
- Errors rejected before fee charging (`BlockhashNotFound`, `AccountNotFound`, `AlreadyProcessed`, signature failures, ...) never land, so they never appear here.
- Not a landing-rate measurement tool.
- No financial or trading features.

## 2. Architecture and layout

Two Cargo workspaces, so on-chain and off-chain dependencies never share a lockfile, toolchain pin or release profile:

```
failscope/
├── Cargo.toml            # off-chain workspace (crates/*), own toolchain + profile
├── crates/
│   ├── decoder/          # pure: logs + err + keys + inner ixs -> DecodedFailure. No I/O.
│   ├── idl/              # fetch + parse + cache Anchor IDLs (legacy + PMP, both formats)
│   ├── ingest/           # Yellowstone client, proto -> decoder input adapter
│   ├── store/            # Store trait + Postgres (sqlx) impl, migrations
│   └── api/              # bin: axum API + static dashboard, wires everything
├── onchain/              # Anchor workspace (Anchor.toml, programs/, toolchain 1.89 pin)
│   └── programs/
│       ├── fail_target/  # renamed from scaffolded `failscope`
│       └── fail_callee/  # tiny program B for nested-CPI failure
├── fixtures/txs/         # getTransaction JSON + expected.json per case
└── docs/decisions.md
```

Data flow: `ingest` (gRPC) → adapter → `decoder` → (`idl` lookup on miss) → `store` → `api`.

Design rule: `decoder` takes a neutral input type (logs, error enum, resolved account keys, inner instructions) and returns a decoded result. No I/O, which makes it easy to test against fixtures. Two adapters build that input type: **RPC JSON** (fixtures) and **Yellowstone protobuf** (ingest). Both get golden tests.

## 3. How errors appear (spec for the decoder)

Verify details against current Solana and Anchor docs before relying on them.

- Transaction metadata carries an error such as `InstructionError(ix_index, Custom(code))` or other variants (e.g. `InstructionError(_, ComputationalBudgetExceeded)`, `InstructionError(_, ProgramFailedToComplete)`, tx-level `InsufficientFundsForRent`, `DuplicateInstruction`). Model the **whole** enum, but note that only fee-charged errors can be observed (see §1). Tx-level errors have no ix index and no failing program, so those fields are nullable.
- `ix_index` refers to the **top-level** instruction. The real failing program is often deeper (CPI).
- Anchor user errors start at 6000. Framework errors use fixed ranges (100+ instruction, 1000+ IDL, 2000+ constraint, 3000+ account, 4100+ misc), decodable with a static table and no IDL. Hex in logs (`0x1771`) = decimal 6001.
- Anchor logs a readable line on failure (error name, number, message, and for constraint errors sometimes the file and line or account). Logs can be **truncated** on busy transactions, so never rely on them alone.
- Log line shapes to parse:
  - `Program <id> invoke [<depth>]`
  - `Program <id> success`
  - `Program <id> failed: <reason>` (e.g. `custom program error: 0x1771`)
  - `Program log: AnchorError ...`
  - `Program <id> consumed <n> of <m> compute units`
  - `Log truncated`

### Decoding pipeline (in order)
1. **Attribute** the failure by walking the log stack (push on `invoke [n]`, pop on `success` / `failed`). Every frame emits `failed:` while unwinding, so the innermost failing program is the **first** `failed` line. Cross-check against the root program and `inner_instructions[ix_index]`. If logs are truncated or absent: no CPI ran → the root program (high); CPIs ran → root as a guess with `attribution_confidence = low`, and no program-keyed name (D13).
2. **Name** the error, first match wins (D12):
   1. Anchor error log line from the failing frame, number matching the code. Source = `anchor_log`.
   2. Native tables by program id: System, SPL Token / Token-2022, Associated Token. Source = `native`.
   3. IDL `errors` of the failing program. Source = `idl`.
   4. Anchor framework table (codes < 6000), only for programs with an IDL. Source = `anchor_framework`.
   5. Non-`Custom` runtime errors, refined by the `failed:` reason (panic, CU exhaustion). Source = `runtime`.
   6. Fallback: keep raw code and program id. Source = `unknown`.

Always record `decode_source` so decoder coverage can be measured and shown in the dashboard.

### Account keys
Full key list = static message keys + `loaded_addresses.writable` + `loaded_addresses.readonly`, **in that order**. Resolve before mapping any program index.

### Derived fields
- `cu_requested`: from a `SetComputeUnitLimit` ix. Otherwise use the runtime default (200k per non-builtin ix, capped at 1.4M; verify current rule).
- `priority_fee`: `SetComputeUnitPrice` (micro-lamports/CU) × CU limit / 1e6, in lamports.

## 4. Data model (starting point)

`failed_tx`
- signature (pk), slot, block_time (nullable), fee, priority_fee (nullable)
- cu_requested, cu_consumed
- top_level_ix_index (nullable)
- failing_program_id (nullable), root_program_id (nullable)
- cpi_depth, attribution_confidence (high | low)
- error_kind (enum), error_code (nullable), error_name (nullable), error_message (nullable), error_account (nullable; account named by an Anchor constraint error)
- decode_source (anchor_log | anchor_framework | idl | native | runtime | unknown)
- signer (fee payer), uses_alt (bool), log_truncated (bool)
- raw_err (jsonb) for debugging

`idl_cache`
- program_id (pk), idl_json, idl_format_version, idl_location (legacy | pmp), fetched_at, fetch_status (found | missing | error)

`ingest_gaps`
- from_slot, to_slot, detected_at, reason

Pick **one** store to start: Postgres (sqlx) now, ClickHouse later if you want an analytics showcase. Keep queries behind a `Store` trait. Use sqlx offline mode (commit `.sqlx/`) so CI runs without a DB.

## 5. Milestones (build in this order)

### M0: Setup
- Move the Anchor scaffold into `onchain/`, rename program `failscope` → `fail_target` (fix `declare_id!`/Anchor.toml), and create the off-chain workspace at root.
- CI (fmt, clippy, test for both workspaces), `tracing` logging, `.env` config + `.env.example`, README skeleton.
- Workspace lints: `[workspace.lints.clippy] unwrap_used = "deny"`, `expect_used = "deny"` (allowed in tests and `main`).
- Done when: `cargo test` and `cargo clippy --all-targets -- -D warnings` pass in both workspaces.

### M1: Companion failing program (Anchor, devnet)
Instructions that each fail deliberately in a distinct way:
1. custom error via `require!` / `err!` (user error, code >= 6000)
2. `has_one` or other constraint violation (2000 range)
3. missing signer: pass the account with `is_signer = false` in the message, so Anchor returns `AccountNotSigner` (3010). Omitting a real signature can't be submitted at all.
4. arithmetic overflow, **unchecked** (`overflow-checks = true` → panic → `ProgramFailedToComplete`-style log). Optionally also a checked variant returning a custom error.
5. compute budget exhaustion (loop)
6. CPI failure: calls the System or Token program with bad inputs (tests attribution to an inner program)
7. nested CPI: `fail_target` calls `fail_callee`, which fails with its own custom error
- Program tests with LiteSVM (already in dev-deps) for fast iteration.
- TypeScript or Rust client script sends each failing transaction on devnet with **`skipPreflight: true`**. Without it, simulation rejects them and they never land.
- Done when: each failure produces a recorded signature listed in `fixtures/README.md`.

### M2: Fixtures
- Fetch each signature with `getTransaction` (JSON, `maxSupportedTransactionVersion: 0`) and save to `fixtures/txs/`.
- ~~Capture at least one failure via Yellowstone gRPC~~: moved to M5 (needs a streaming endpoint).
- Add a few real **mainnet** failed transactions (slippage failure on a DEX/aggregator, a compute-budget failure, one using an ALT, one with truncated logs if findable) with hand-verified expected results.
- Done when: each fixture has an `expected.json` (failing program, error name, code, decode source, attribution confidence).

### M3: Decoder crate (core milestone)
- Neutral input type + RPC JSON adapter.
- Log stack parser and attribution, cross-checked with inner instructions.
- Anchor error line parser + Anchor framework error table.
- Full `TransactionError` / `InstructionError` modeling.
- Native decoders (System, Token, Token-2022, ATA, compute budget), keyed by (program_id, code).
- Golden tests: every fixture decodes to its expected output. Property or fuzz tests on the log parser (malformed and truncated logs must never panic).
- Done when: all fixtures pass; truncated-log case is handled and flagged.

### M4: IDL crate
- Fetch the on-chain IDL from **both** locations:
  - legacy Anchor IDL account (seed-derived from the program id, zlib-compressed)
  - Program Metadata Program account (newer Anchor versions upload here; verify)
- Support both the legacy IDL format and the 0.30+ spec format.
- Cache results (including negative results) with a TTL; handle programs with no IDL gracefully.
- Done when: `fail_target` errors decode via IDL with the log fallback disabled in a test.

### M5: Ingest
- Capture at least one failure as a raw Yellowstone message and add it as a fixture, so the proto adapter has golden coverage.
- Yellowstone gRPC client subscribing to transactions with `failed: true`, `vote: false` (verify current proto fields).
- Also subscribe to `blocks_meta` to fill `block_time` (tx updates don't carry it); leave nullable if missing.
- Proto → decoder input adapter, golden-tested with the M2 gRPC fixture.
- Reconnect with backoff, resume via `from_slot` from the last processed slot, bounded channels for backpressure, graceful shutdown.
- Commitment: `confirmed` (lower latency than `finalized`; reorg risk is negligible in practice). Document why in `docs/decisions.md`.
- Idempotent inserts (signature pk, `ON CONFLICT DO NOTHING`).
- Done when: running against a Yellowstone endpoint (local validator + plugin, per D17; a devnet/mainnet provider only changes the endpoint) inserts decoded failures into the DB; killing the connection resumes via `from_slot` with no duplicates when inside the replay window, and otherwise records the missed slot range in `ingest_gaps`. Verified by `dev/e2e-resume.sh` and `dev/e2e-gap.sh`.

### M6: Store and API
- (M5 already added the Postgres store: migrations, idempotent insert, cursor, gaps.)
- Persist the IDL cache (`idl_cache` table).
- The key queries:
  - top failing programs, last N hours
  - top error names for a program
  - failure counts over time (bucketed)
  - decode coverage by `decode_source`
- axum endpoints with simple pagination, JSON only, plus OpenAPI or a documented README table. (Done: `docs/api.md`, D20.)

### M7: Dashboard
- One good page: top failing programs and error names over the last 24h, a time-series chart, and a program detail view. Keep it plain (static HTML + a chart lib, or htmx).
- Done when: you can point a screenshot at the README. (Done: `docs/dashboard.png`, D22.)

### M8: Stretch (pick one)
- ~~Bot-likelihood heuristic (e.g. signer failure rate, tx frequency, tip patterns) with a documented, honest methodology.~~ Dropped (D24): only meaningful on real mainnet traffic.
- Alerting (webhook) on error spikes for a chosen program. **(Chosen and done: `failscope alert`, D23.)**
- ClickHouse backend behind the store trait.

## 6. Suggested crates (verify current versions)

tokio, tonic (via yellowstone), `yellowstone-grpc-client` and `yellowstone-grpc-proto`, the split `solana-*` crates (`solana-pubkey`, `solana-transaction-error`, `solana-instruction`, ...; keep dependencies light and versions compatible with the yellowstone release), serde / serde_json, sqlx (postgres), axum, tower-http, clap, tracing / tracing-subscriber, thiserror / anyhow, `regex` or hand-written parsing for logs, `flate2` (IDL decompression), `proptest` or `cargo-fuzz`.

## 7. Gotchas checklist

- Versioned transactions and address lookup tables: resolve static + loaded writable + loaded readonly keys, in that order, before mapping program indexes.
- Inner instructions are grouped by top-level index; use them to corroborate log-based attribution.
- Logs may be truncated, absent, or from non-Anchor programs.
- `Custom(u32)` codes mean different things per program, so always key decoding by (program_id, code).
- Same signature can arrive more than once on reconnect; inserts must be idempotent.
- Yellowstone tx updates have no `block_time`; `from_slot` replay is bounded by provider retention.
- Slot and commitment: processed vs confirmed vs finalized affects reorg risk. Document the choice.
- Provider streams disconnect and rate limit; test the reconnect path deliberately.
- Failing test txs need `skipPreflight: true`.
- Don't leak API tokens into the repo; use env vars and `.env.example`.

## 8. Cost and environment notes

- Yellowstone access usually comes from a paid RPC provider (entry plans exist, but pricing changes, so check current plans) or a self-run node. For development, prefer **devnet** plus recorded fixtures so most of the work needs no streaming plan.
- ~~Only buy mainnet streaming access when you reach M5 and want real data.~~ Scope decision (2026-10-10, D24): devnet and local validator only; no mainnet streaming.

## 9. README requirements (this is what hiring managers will read)

- One-paragraph pitch plus a screenshot.
- "Failed vs dropped" explanation and the limits of the tool.
- Architecture diagram and decoding pipeline description.
- A table of every failure mode in the companion program and how the decoder classifies it.
- Decoder coverage numbers from a real run.
- A short "what I learned / what surprised me" section, e.g. CPI attribution pitfalls.

## 10. Working agreement for Claude Code

- Work milestone by milestone; do not start the next until the current "done when" is met.
- Tests first for the decoder crate. No `unwrap()` / `expect()` outside tests and main (enforced by clippy lints).
- Keep `decoder` free of network and DB dependencies.
- After each milestone: run fmt, clippy, tests; summarize what changed and what is left.
- When a spec detail is marked "verify", check current docs and note the finding in `docs/decisions.md`.
