//! Adapter: `getTransaction` JSON (`encoding: "json"`) to [`TxInput`].

use serde::Deserialize;
use solana_transaction_error::TransactionError;

use crate::input::{CompiledInstruction, InnerInstruction, InnerInstructions, TxInput};

#[derive(Debug, thiserror::Error)]
pub enum RpcAdapterError {
    #[error("invalid getTransaction JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("transaction {0} did not fail")]
    NotFailed(String),
    #[error("transaction has no signature")]
    NoSignature,
    #[error("instruction data is not base58: {0}")]
    Data(#[from] bs58::decode::Error),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcTransaction {
    slot: u64,
    block_time: Option<i64>,
    transaction: RpcInnerTx,
    meta: RpcMeta,
}

#[derive(Deserialize)]
struct RpcInnerTx {
    signatures: Vec<String>,
    message: RpcMessage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcMessage {
    account_keys: Vec<String>,
    header: RpcHeader,
    instructions: Vec<RpcInstruction>,
    #[serde(default)]
    address_table_lookups: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcHeader {
    num_required_signatures: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcInstruction {
    program_id_index: u8,
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcMeta {
    err: Option<TransactionError>,
    fee: u64,
    #[serde(default)]
    inner_instructions: Option<Vec<RpcInnerGroup>>,
    #[serde(default)]
    log_messages: Option<Vec<String>>,
    #[serde(default)]
    loaded_addresses: Option<RpcLoaded>,
    #[serde(default)]
    compute_units_consumed: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcInnerGroup {
    index: u8,
    instructions: Vec<RpcInnerInstruction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcInnerInstruction {
    program_id_index: u8,
    #[serde(default)]
    stack_height: Option<u32>,
}

#[derive(Deserialize)]
struct RpcLoaded {
    writable: Vec<String>,
    readonly: Vec<String>,
}

/// Parses the `result` of `getTransaction`. Errors if the transaction succeeded.
pub fn from_rpc_json(value: &serde_json::Value) -> Result<TxInput, RpcAdapterError> {
    let rpc = RpcTransaction::deserialize(value)?;
    let signature = rpc
        .transaction
        .signatures
        .first()
        .cloned()
        .ok_or(RpcAdapterError::NoSignature)?;
    let err = rpc
        .meta
        .err
        .ok_or_else(|| RpcAdapterError::NotFailed(signature.clone()))?;

    let message = rpc.transaction.message;
    let mut account_keys = message.account_keys;
    if let Some(loaded) = rpc.meta.loaded_addresses {
        account_keys.extend(loaded.writable);
        account_keys.extend(loaded.readonly);
    }

    let instructions = message
        .instructions
        .into_iter()
        .map(|ix| {
            Ok(CompiledInstruction {
                program_id_index: ix.program_id_index,
                data: bs58::decode(&ix.data).into_vec()?,
            })
        })
        .collect::<Result<Vec<_>, RpcAdapterError>>()?;

    let inner_instructions = rpc
        .meta
        .inner_instructions
        .unwrap_or_default()
        .into_iter()
        .map(|g| InnerInstructions {
            index: g.index,
            instructions: g
                .instructions
                .into_iter()
                .map(|ix| InnerInstruction {
                    program_id_index: ix.program_id_index,
                    stack_height: ix.stack_height,
                })
                .collect(),
        })
        .collect();

    Ok(TxInput {
        signature,
        slot: rpc.slot,
        block_time: rpc.block_time,
        fee: rpc.meta.fee,
        err,
        account_keys,
        num_signatures: message.header.num_required_signatures,
        instructions,
        inner_instructions,
        logs: rpc.meta.log_messages,
        cu_consumed: rpc.meta.compute_units_consumed,
        uses_alt: message.address_table_lookups.is_some_and(|l| !l.is_empty()),
    })
}
