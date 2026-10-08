# Fixtures

## Companion program failures (devnet)

Sent by `onchain/client` (`cargo run -p fail_client --bin send_failures`) against the deployed programs:

- `fail_target`: `6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey`
- `fail_callee`: `BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb`

All were sent with `skip_preflight` and confirmed as failed on chain. Full signatures are in [`signatures.json`](signatures.json). The same cases run locally in `onchain/client/tests/litesvm.rs`, which asserts the error and the innermost `failed:` log line for each.

| # | Case | Instruction | Transaction error | Innermost failing program | Distinguishing log | Tx |
|---|---|---|---|---|---|---|
| 1 | `custom_error` | `fail_custom`: `require!` user error | `InstructionError(0, Custom(6000))` | 6MzbWC… | `AnchorError thrown in …` AlwaysFails / 6000 | [5WAFxXMn…](https://explorer.solana.com/tx/5WAFxXMnhR22RcddzQF8BmJ234WvB2V7RsWG9nSWpq2wsVz3h7No4ZLpZjFzetTFcKJ9f6MLtSAvqnqAboQ5s7w3?cluster=devnet) |
| 2 | `has_one_violation` | `fail_has_one`: impostor signs as authority | `InstructionError(0, Custom(2001))` | 6MzbWC… | `AnchorError caused by account: counter` ConstraintHasOne / 2001, then `Left:`/`Right:` lines | [2KGfeQ9H…](https://explorer.solana.com/tx/2KGfeQ9Hkfkz92xSKVuf6tW2MvA5UfB9xNm4QPduoKDKApK6SyxifsHyaikuLBd3j6PmELhu2AHpQCVMzgcT3NTZ?cluster=devnet) |
| 3 | `missing_signer` | `fail_missing_signer`: authority passed with `is_signer = false` | `InstructionError(0, Custom(3010))` | 6MzbWC… | `AnchorError caused by account: authority` AccountNotSigner / 3010 | [4MGWvRg3…](https://explorer.solana.com/tx/4MGWvRg3R7rmpJ6H9yb3hmfJ7n5H2akzWSiQjnYn67zPENdHPngwtpyBZpJyz2RKLvpxQwbrEDLjdKjLC7o773pC?cluster=devnet) |
| 4a | `overflow_panic` | `fail_overflow_panic`: unchecked `u64::MAX + 1` | `InstructionError(0, ProgramFailedToComplete)` | 6MzbWC… | `failed: SBF program Panicked in …`; no AnchorError | [5HWjSnLZ…](https://explorer.solana.com/tx/5HWjSnLZGdCNWW6sHeSzG6FBXnTVbZfTzjdsTmC6n73zLJ8y9vRQXfZePfxN9RjsdG7TAb1Sh43fPjq2QKKoFfVE?cluster=devnet) |
| 4b | `overflow_checked` | `fail_overflow_checked`: `checked_add` → user error | `InstructionError(0, Custom(6001))` | 6MzbWC… | `AnchorError occurred` CheckedOverflow / 6001 | [3SJoqA1K…](https://explorer.solana.com/tx/3SJoqA1KXmkZK4XN2ZacjR5pz1bXdtZvGwYPqDBgtpsbLwWNW8h1bvBEHMUoyUQ4jwmNC6TnXDY18Sqf1posVSx8?cluster=devnet) |
| 5 | `compute_exhausted` | `fail_compute`: busy loop, CU limit 20k | `InstructionError(1, ProgramFailedToComplete)` | 6MzbWC… | `failed: exceeded CUs meter at BPF instruction` | [P7bsBzJw…](https://explorer.solana.com/tx/P7bsBzJwnJiMeW7YsivhkUwCT5BMnJGniLS3PGpHEY5vH9ev9iRBr52ZFaJqwjbNqpYebSAt3K58yr67ABXGi3V?cluster=devnet) |
| 6 | `cpi_system_transfer` | `fail_cpi_system`: CPI transfer of `u64::MAX` lamports | `InstructionError(0, Custom(1))` | System | System `failed: custom program error: 0x1`, then fail_target re-reports 0x1 | [LmKPNBCz…](https://explorer.solana.com/tx/LmKPNBCz59Vt8coAyMYWPnPuBx2YdeGhLNzNtDZbZpSyZsiNsKcjRZ87Kjqm1SfyHU9SL7dKw4GtWJ1zYpK1TtE?cluster=devnet) |
| 7 | `nested_cpi_callee` | `fail_nested_cpi`: CPI into `fail_callee` | `InstructionError(0, Custom(6000))` | BCUANL… | callee AnchorError CalleeAlwaysFails / 6000, then fail_target re-reports 0x1770 | [23BA6WjR…](https://explorer.solana.com/tx/23BA6WjR3b2eaFqgY7g7mnRma3Tbd6q8LYzYeJaVt9Duiq6167DHaZsPK7WkCWEYQLmSvjoStKgaxTabDLGAwh6i?cluster=devnet) |

### Traps these cases exercise

- **#1 vs #7:** identical top-level error `InstructionError(0, Custom(6000))`, but different programs and different meanings. Decoding by the top-level program alone gets #7 wrong.
- **#6:** the error code `1` belongs to the System program (`ResultWithNegativeLamports`), not to `fail_target`, even though `ix_index` 0 points at `fail_target`.
- **#4a vs #5:** both are `ProgramFailedToComplete`. Only the `failed:` log line tells a panic from compute exhaustion. The runtime does **not** report `ComputationalBudgetExceeded` for running out of CUs.
- **#5:** `ix_index` is 1, because the compute-budget instruction comes first.

## Decoder fixtures (`txs/`)

One directory per fixture:

- `tx.json`: the `result` of `getTransaction` with `encoding: "json"`, `maxSupportedTransactionVersion: 0`, `commitment: "confirmed"`. Fetch or refresh with `scripts/fetch-fixture.sh <name> <signature> [rpc_url]`.
- `expected.json`: what the decoder must output. Hand-verified against the logs, and against the program's IDL where one exists.

| Fixture | Cluster | Failing program (depth) | Expected | Source | Why it's here |
|---|---|---|---|---|---|
| `devnet_*` (8) | devnet | see table above | see table above | anchor_log / runtime / native | Every companion-program failure mode |
| `mainnet_jupiter_slippage` | mainnet | Jupiter v6 (1) | 6001 SlippageToleranceExceeded | idl | Real slippage failure. **No AnchorError log line**, so only the IDL can name it |
| `mainnet_jupiter_panic` | mainnet | Jupiter v6 (1) | ProgramPanicked | runtime | Real panic in a production program |
| `mainnet_compute_exhausted` | mainnet | Jupiter v6 (2) | ComputeUnitsExceeded | runtime | CUs run out in a self-CPI. The outer frame logs a *different* reason (`Program failed to complete`) |
| `mainnet_cpi_inner_custom` | mainnet | `HBVw6…` (2) | code 8, name unknown | unknown | Top-level program is Jupiter but the failure is in an inner, silent program with no IDL |

All four mainnet fixtures are v0 transactions with address lookup tables. Account keys must be resolved through `meta.loadedAddresses`.

### `expected.json` fields

| Field | Meaning |
|---|---|
| `top_level_ix_index` | Index from `InstructionError(index, _)` |
| `root_program_id` | Program of that top-level instruction |
| `failing_program_id` | Innermost program that failed (first `failed:` log line) |
| `cpi_depth` | Invoke depth of the failing program (1 = top level) |
| `error_kind` | snake_case `InstructionError` variant (`custom`, `program_failed_to_complete`, ...), or the `TransactionError` variant for tx-level errors |
| `error_code` | `Custom(n)` code, else null |
| `error_name` / `error_message` | Decoded name / message. Runtime failures use `ProgramPanicked` / `ComputeUnitsExceeded` |
| `decode_source` | `anchor_log`, `anchor_framework`, `idl`, `native`, `runtime` or `unknown` |
| `attribution_confidence` | `high` when the log stack identifies the failing program, `low` when logs are missing or truncated |
| `signer`, `uses_alt`, `log_truncated`, `cu_consumed`, `fee` | Copied from the transaction |

Golden tests compare every field exactly, except `error_message`. That field is compared only when it is non-null, because the message text for `native` and `runtime` errors is a presentation choice made in M3.

## IDLs (`idls/`)

On-chain IDLs saved for offline tests, named `<program_id>.json`:

- `JUP6Lkb….json`: Jupiter v6, read from the **legacy** Anchor IDL account `C88XWfp26heEmDkmfSzeXP7Fd7GQJ2j9dDTUsyiZbUTa` (zlib-decompressed). It's in the new spec format (`metadata.spec` 0.1.0).

## Not covered yet

- **Truncated logs:** none found in about 120 scanned mainnet failures. M3 builds this case by truncating a real fixture's logs.
- **SPL Token as the failing program:** none found in the samples scanned (public RPC rate limits cut scans short). Native decoding is covered by the System program case.
- **Yellowstone gRPC capture:** needs a streaming endpoint. Moved to M5.
