//! Adapter: Yellowstone `SubscribeUpdateTransactionInfo` to the decoder's
//! [`TxInput`]. The RPC adapter lives in the decoder; tests check both
//! produce the same input for the same transaction.

use failscope_decoder::{
    CompiledInstruction, InnerInstruction, InnerInstructions, TransactionError, TxInput,
};
use yellowstone_grpc_proto::prelude::SubscribeUpdateTransactionInfo;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("update has no {0}")]
    Missing(&'static str),
    #[error("{0} out of range: {1}")]
    Range(&'static str, u32),
    #[error("transaction error bytes are not a bincode TransactionError: {0}")]
    Error(#[from] bincode::Error),
}

fn to_u8(what: &'static str, v: u32) -> Result<u8, ConvertError> {
    u8::try_from(v).map_err(|_| ConvertError::Range(what, v))
}

fn b58(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

/// Converts one transaction update. `Ok(None)` if the transaction succeeded.
pub fn tx_input(
    slot: u64,
    info: &SubscribeUpdateTransactionInfo,
) -> Result<Option<TxInput>, ConvertError> {
    let meta = info.meta.as_ref().ok_or(ConvertError::Missing("meta"))?;
    let Some(err) = meta.err.as_ref() else {
        return Ok(None);
    };
    // Agave stores the error as bincode, the same bytes the RPC turns into JSON.
    let err: TransactionError = bincode::deserialize(&err.err)?;

    let tx = info
        .transaction
        .as_ref()
        .ok_or(ConvertError::Missing("transaction"))?;
    let message = tx
        .message
        .as_ref()
        .ok_or(ConvertError::Missing("message"))?;
    let header = message
        .header
        .as_ref()
        .ok_or(ConvertError::Missing("header"))?;

    let account_keys = message
        .account_keys
        .iter()
        .chain(&meta.loaded_writable_addresses)
        .chain(&meta.loaded_readonly_addresses)
        .map(|k| b58(k))
        .collect();

    let instructions = message
        .instructions
        .iter()
        .map(|ix| {
            Ok(CompiledInstruction {
                program_id_index: to_u8("program_id_index", ix.program_id_index)?,
                data: ix.data.clone(),
            })
        })
        .collect::<Result<_, ConvertError>>()?;

    let inner_instructions = if meta.inner_instructions_none {
        Vec::new()
    } else {
        meta.inner_instructions
            .iter()
            .map(|group| {
                Ok(InnerInstructions {
                    index: to_u8("inner index", group.index)?,
                    instructions: group
                        .instructions
                        .iter()
                        .map(|ix| {
                            Ok(InnerInstruction {
                                program_id_index: to_u8("program_id_index", ix.program_id_index)?,
                                stack_height: ix.stack_height,
                            })
                        })
                        .collect::<Result<_, ConvertError>>()?,
                })
            })
            .collect::<Result<_, ConvertError>>()?
    };

    Ok(Some(TxInput {
        signature: b58(&info.signature),
        slot,
        // Not in transaction updates; filled from block meta later.
        block_time: None,
        fee: meta.fee,
        err,
        account_keys,
        num_signatures: to_u8("num_required_signatures", header.num_required_signatures)?,
        instructions,
        inner_instructions,
        logs: (!meta.log_messages_none).then(|| meta.log_messages.clone()),
        cu_consumed: meta.compute_units_consumed,
        uses_alt: !message.address_table_lookups.is_empty(),
    }))
}
