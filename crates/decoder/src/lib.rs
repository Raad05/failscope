//! Pure decoder: turns failed-transaction data into a classified, attributed
//! failure. No I/O: callers pass a [`TxInput`] and IDL error tables.
//!
//! Pipeline (see `decode`):
//! 1. Attribute the failure to the innermost failing program via the log stack,
//!    cross-checked with inner instructions.
//! 2. Name the error, first match wins: Anchor log line, native program table,
//!    IDL, Anchor framework table, runtime variant, else unknown.

mod anchor_log;
mod compute;
mod decode;
mod idl;
mod input;
mod logs;
mod native;
mod native_tables;
mod rpc;

pub use anchor_log::{parse_anchor_error, AnchorErrorLog};
pub use compute::{compute_budget, ComputeBudget};
pub use decode::{decode, Confidence, DecodeSource, DecodedFailure};
pub use idl::{ErrorLookup, IdlError, IdlErrorTable, NoIdls};
pub use input::{CompiledInstruction, InnerInstruction, InnerInstructions, TxInput};
pub use logs::{analyze_logs, FrameFailure, LogAnalysis};
pub use native::{anchor_framework_error, native_error, KnownError};
pub use rpc::{from_rpc_json, RpcAdapterError};
pub use solana_instruction_error::InstructionError;
pub use solana_transaction_error::TransactionError;
