//! The decoding pipeline: attribute the failure to a program, then name it.

use serde::Serialize;
use solana_instruction_error::InstructionError;
use solana_transaction_error::TransactionError;

use crate::compute::compute_budget;
use crate::idl::ErrorLookup;
use crate::input::TxInput;
use crate::logs::{analyze_logs, FrameFailure, LogAnalysis};
use crate::native::{anchor_framework_error, native_error};

/// Where the error's name came from. Recorded so coverage can be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeSource {
    /// Anchor's `AnchorError` log line from the failing frame.
    AnchorLog,
    /// Anchor framework error table (codes < 6000), for programs with an IDL.
    AnchorFramework,
    /// The failing program's IDL.
    Idl,
    /// Built-in tables for System, SPL Token, Token-2022, ATA.
    Native,
    /// Non-`Custom` errors defined by the runtime (panic, CU exhaustion, ...).
    Runtime,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// The failing program is known: from a consistent log stack, or because
    /// no CPI ran so only the top-level program could have failed.
    High,
    /// The failing program is a guess (logs missing/truncated and CPIs ran,
    /// or the log stack disagrees with the transaction).
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecodedFailure {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub fee: u64,
    pub priority_fee: u64,
    pub cu_requested: u32,
    pub cu_consumed: Option<u64>,
    /// Fee payer.
    pub signer: Option<String>,
    pub uses_alt: bool,
    pub log_truncated: bool,
    /// `None` for transaction-level errors.
    pub top_level_ix_index: Option<u8>,
    pub root_program_id: Option<String>,
    pub failing_program_id: Option<String>,
    /// Invoke depth of the failing program (top level = 1), when known.
    pub cpi_depth: Option<u32>,
    pub attribution_confidence: Confidence,
    /// snake_case `InstructionError` variant, or `TransactionError` variant
    /// for transaction-level errors.
    pub error_kind: String,
    pub error_code: Option<u32>,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    /// Account named by an Anchor constraint error.
    pub error_account: Option<String>,
    pub decode_source: DecodeSource,
    /// The transaction error as the RPC serializes it.
    pub raw_err: serde_json::Value,
}

struct Attribution {
    failing_program_id: Option<String>,
    cpi_depth: Option<u32>,
    confidence: Confidence,
    frame: Option<FrameFailure>,
}

struct Naming {
    name: Option<String>,
    message: Option<String>,
    account: Option<String>,
    source: DecodeSource,
}

impl Naming {
    fn unknown() -> Self {
        Naming {
            name: None,
            message: None,
            account: None,
            source: DecodeSource::Unknown,
        }
    }

    fn known(name: &str, message: Option<&str>, source: DecodeSource) -> Self {
        Naming {
            name: Some(name.to_string()),
            message: message.map(str::to_string),
            account: None,
            source,
        }
    }
}

pub fn decode(tx: &TxInput, idls: &dyn ErrorLookup) -> DecodedFailure {
    let analysis = tx.logs.as_deref().map(analyze_logs).unwrap_or_default();
    let raw_err = serde_json::to_value(&tx.err).unwrap_or(serde_json::Value::Null);
    let compute = compute_budget(tx);

    let (ix_index, ix_err) = match &tx.err {
        TransactionError::InstructionError(i, e) => (Some(*i), Some(e)),
        _ => (None, None),
    };
    let root_program_id = ix_index
        .and_then(|i| tx.top_level_program(i))
        .map(str::to_string);

    let attribution = match ix_index {
        Some(i) => attribute(tx, i, root_program_id.as_deref(), &analysis),
        None => Attribution {
            failing_program_id: None,
            cpi_depth: None,
            confidence: Confidence::High,
            frame: None,
        },
    };

    let (error_kind, error_code, naming) = match ix_err {
        Some(InstructionError::Custom(code)) => (
            "custom".to_string(),
            Some(*code),
            name_custom(*code, &attribution, idls),
        ),
        Some(other) => (
            variant_kind(&raw_err, true),
            None,
            name_runtime_ix(other, attribution.frame.as_ref()),
        ),
        None => (
            variant_kind(&raw_err, false),
            None,
            Naming::known(
                &pascal_variant(&raw_err, false),
                Some(&tx.err.to_string()),
                DecodeSource::Runtime,
            ),
        ),
    };

    DecodedFailure {
        signature: tx.signature.clone(),
        slot: tx.slot,
        block_time: tx.block_time,
        fee: tx.fee,
        priority_fee: compute.priority_fee,
        cu_requested: compute.cu_requested,
        cu_consumed: tx.cu_consumed,
        signer: tx.account_keys.first().cloned(),
        uses_alt: tx.uses_alt,
        log_truncated: analysis.truncated,
        top_level_ix_index: ix_index,
        root_program_id,
        failing_program_id: attribution.failing_program_id,
        cpi_depth: attribution.cpi_depth,
        attribution_confidence: attribution.confidence,
        error_kind,
        error_code,
        error_name: naming.name,
        error_message: naming.message,
        error_account: naming.account,
        decode_source: naming.source,
        raw_err,
    }
}

fn attribute(tx: &TxInput, ix_index: u8, root: Option<&str>, logs: &LogAnalysis) -> Attribution {
    let cpis_ran = tx
        .inner_for(ix_index)
        .is_some_and(|g| !g.instructions.is_empty());

    if let Some(frame) = &logs.failure {
        let root_matches = match (frame.root_program_id.as_deref(), root) {
            (Some(log_root), Some(root)) => log_root == root,
            _ => true,
        };
        // A failure below the top level should appear among the recorded
        // inner instructions of that top-level instruction.
        let inner_agrees = frame.depth <= 1
            || tx.inner_for(ix_index).is_none_or(|g| {
                g.instructions
                    .iter()
                    .any(|ix| tx.key(ix.program_id_index) == Some(frame.program_id.as_str()))
            });
        let confidence = if frame.stack_consistent && root_matches && inner_agrees {
            Confidence::High
        } else {
            Confidence::Low
        };
        return Attribution {
            failing_program_id: Some(frame.program_id.clone()),
            cpi_depth: Some(frame.depth),
            confidence,
            frame: Some(frame.clone()),
        };
    }

    if cpis_ran {
        // Logs can't say which frame failed, and more than one program ran.
        Attribution {
            failing_program_id: root.map(str::to_string),
            cpi_depth: None,
            confidence: Confidence::Low,
            frame: None,
        }
    } else {
        // No CPI ran, so only the top-level program can have failed.
        Attribution {
            failing_program_id: root.map(str::to_string),
            cpi_depth: Some(1),
            confidence: Confidence::High,
            frame: None,
        }
    }
}

fn name_custom(code: u32, attribution: &Attribution, idls: &dyn ErrorLookup) -> Naming {
    // A guessed program would give a confident-looking but possibly wrong
    // name, since codes are only meaningful per program.
    if attribution.confidence == Confidence::Low && attribution.frame.is_none() {
        return Naming::unknown();
    }
    let Some(program) = attribution.failing_program_id.as_deref() else {
        return Naming::unknown();
    };

    if let Some(anchor) = attribution
        .frame
        .as_ref()
        .and_then(|f| f.anchor_error.as_ref())
        .filter(|a| a.number == code)
    {
        return Naming {
            name: Some(anchor.name.clone()),
            message: Some(anchor.message.clone()),
            account: anchor.account.clone(),
            source: DecodeSource::AnchorLog,
        };
    }
    if let Some(e) = native_error(program, code) {
        return Naming::known(e.name, Some(e.message), DecodeSource::Native);
    }
    if let Some(e) = idls.idl_error(program, code) {
        return Naming::known(&e.name, e.message.as_deref(), DecodeSource::Idl);
    }
    if idls.has_idl(program) {
        if let Some(e) = anchor_framework_error(code) {
            return Naming::known(e.name, Some(e.message), DecodeSource::AnchorFramework);
        }
    }
    Naming::unknown()
}

fn name_runtime_ix(err: &InstructionError, frame: Option<&FrameFailure>) -> Naming {
    let reason = frame.map(|f| f.reason.as_str());
    let name = match (err, reason) {
        (InstructionError::ProgramFailedToComplete, Some(r)) if r.contains("Panicked") => {
            "ProgramPanicked".to_string()
        }
        (InstructionError::ProgramFailedToComplete, Some(r))
            if r.contains("exceeded CUs meter") =>
        {
            "ComputeUnitsExceeded".to_string()
        }
        _ => {
            let value = serde_json::to_value(err).unwrap_or(serde_json::Value::Null);
            variant_name(&value).unwrap_or_else(|| "Unknown".to_string())
        }
    };
    let message = reason.map_or_else(|| err.to_string(), str::to_string);
    Naming {
        name: Some(name),
        message: Some(message),
        account: None,
        source: DecodeSource::Runtime,
    }
}

/// Variant name of a serde-externally-tagged enum value: `"Foo"` or `{"Foo": ...}`.
fn variant_name(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) if map.len() == 1 => map.keys().next().cloned(),
        _ => None,
    }
}

/// For `InstructionError(i, inner)` the inner variant, else the outer one.
fn pascal_variant(raw_err: &serde_json::Value, inner: bool) -> String {
    let value = if inner {
        raw_err
            .get("InstructionError")
            .and_then(|v| v.get(1))
            .unwrap_or(raw_err)
    } else {
        raw_err
    };
    variant_name(value).unwrap_or_else(|| "Unknown".to_string())
}

fn variant_kind(raw_err: &serde_json::Value, inner: bool) -> String {
    to_snake_case(&pascal_variant(raw_err, inner))
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case() {
        assert_eq!(
            to_snake_case("ProgramFailedToComplete"),
            "program_failed_to_complete"
        );
        assert_eq!(to_snake_case("Custom"), "custom");
    }

    #[test]
    fn kinds_from_raw_errors() {
        let ix = serde_json::to_value(TransactionError::InstructionError(
            3,
            InstructionError::InvalidAccountData,
        ))
        .unwrap();
        assert_eq!(variant_kind(&ix, true), "invalid_account_data");
        let tx =
            serde_json::to_value(TransactionError::InsufficientFundsForRent { account_index: 2 })
                .unwrap();
        assert_eq!(variant_kind(&tx, false), "insufficient_funds_for_rent");
    }
}
