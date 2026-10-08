//! The decoder's neutral input. Adapters (RPC JSON here, Yellowstone protobuf
//! in `ingest`) build a [`TxInput`]; the decoder never sees wire formats.

use solana_transaction_error::TransactionError;

/// A failed transaction, reduced to what decoding needs.
#[derive(Debug, Clone, PartialEq)]
pub struct TxInput {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub fee: u64,
    pub err: TransactionError,
    /// Full account list: static keys, then loaded writable, then loaded
    /// readonly addresses (the order instruction indexes refer to).
    pub account_keys: Vec<String>,
    pub num_signatures: u8,
    pub instructions: Vec<CompiledInstruction>,
    pub inner_instructions: Vec<InnerInstructions>,
    /// `None` when the source had no logs at all (not the same as empty).
    pub logs: Option<Vec<String>>,
    pub cu_consumed: Option<u64>,
    pub uses_alt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_id_index: u8,
    pub data: Vec<u8>,
}

/// Inner (CPI) instructions recorded under one top-level instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerInstructions {
    pub index: u8,
    pub instructions: Vec<InnerInstruction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerInstruction {
    pub program_id_index: u8,
    /// Invoke depth (top level = 1). Missing on old transactions.
    pub stack_height: Option<u32>,
}

impl TxInput {
    pub fn key(&self, index: u8) -> Option<&str> {
        self.account_keys
            .get(usize::from(index))
            .map(String::as_str)
    }

    /// Program id of the top-level instruction at `ix_index`.
    pub fn top_level_program(&self, ix_index: u8) -> Option<&str> {
        let ix = self.instructions.get(usize::from(ix_index))?;
        self.key(ix.program_id_index)
    }

    pub fn inner_for(&self, ix_index: u8) -> Option<&InnerInstructions> {
        self.inner_instructions.iter().find(|g| g.index == ix_index)
    }
}
