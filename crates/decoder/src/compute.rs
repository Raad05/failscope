//! Requested compute units and priority fee, from the transaction's
//! compute-budget instructions. Rules follow agave 3.1
//! (`solana-compute-budget-instruction`, `solana-compute-budget`).

use crate::input::TxInput;
use crate::native::{BUILTIN_PROGRAMS, COMPUTE_BUDGET_PROGRAM};

pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
pub const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;
pub const MAX_BUILTIN_ALLOCATION_COMPUTE_UNIT_LIMIT: u32 = 3_000;
const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

const SET_COMPUTE_UNIT_LIMIT: u8 = 2;
const SET_COMPUTE_UNIT_PRICE: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeBudget {
    /// CU limit in force: explicit `SetComputeUnitLimit`, else the default.
    pub cu_requested: u32,
    /// Micro-lamports per CU from `SetComputeUnitPrice` (0 if none).
    pub cu_price: u64,
    /// `ceil(cu_price * cu_requested / 1e6)` lamports.
    pub priority_fee: u64,
}

pub fn compute_budget(tx: &TxInput) -> ComputeBudget {
    let mut explicit_limit = None;
    let mut cu_price = 0;
    let mut builtin = 0u32;
    let mut non_builtin = 0u32;

    for ix in &tx.instructions {
        let program = tx.key(ix.program_id_index).unwrap_or_default();
        if BUILTIN_PROGRAMS.contains(&program) {
            builtin = builtin.saturating_add(1);
        } else {
            non_builtin = non_builtin.saturating_add(1);
        }
        if program != COMPUTE_BUDGET_PROGRAM {
            continue;
        }
        match ix.data.split_first() {
            Some((&SET_COMPUTE_UNIT_LIMIT, rest)) => {
                if let Some(bytes) = rest.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok()) {
                    explicit_limit = Some(u32::from_le_bytes(bytes));
                }
            }
            Some((&SET_COMPUTE_UNIT_PRICE, rest)) => {
                if let Some(bytes) = rest.get(..8).and_then(|b| <[u8; 8]>::try_from(b).ok()) {
                    cu_price = u64::from_le_bytes(bytes);
                }
            }
            _ => {}
        }
    }

    let cu_requested = explicit_limit
        .unwrap_or_else(|| {
            builtin
                .saturating_mul(MAX_BUILTIN_ALLOCATION_COMPUTE_UNIT_LIMIT)
                .saturating_add(non_builtin.saturating_mul(DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT))
        })
        .min(MAX_COMPUTE_UNIT_LIMIT);

    ComputeBudget {
        cu_requested,
        cu_price,
        priority_fee: priority_fee(cu_price, cu_requested),
    }
}

fn priority_fee(cu_price: u64, cu_limit: u32) -> u64 {
    let micro = u128::from(cu_price) * u128::from(cu_limit);
    let lamports = micro.div_ceil(MICRO_LAMPORTS_PER_LAMPORT);
    u64::try_from(lamports).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::CompiledInstruction;
    use solana_transaction_error::TransactionError;

    fn tx(ixs: Vec<(&str, Vec<u8>)>) -> TxInput {
        let mut keys = vec!["payer".to_string()];
        let instructions = ixs
            .into_iter()
            .map(|(program, data)| {
                keys.push(program.to_string());
                CompiledInstruction {
                    program_id_index: u8::try_from(keys.len() - 1).unwrap(),
                    data,
                }
            })
            .collect();
        TxInput {
            signature: String::new(),
            slot: 0,
            block_time: None,
            fee: 0,
            err: TransactionError::AccountInUse,
            account_keys: keys,
            num_signatures: 1,
            instructions,
            inner_instructions: vec![],
            logs: None,
            cu_consumed: None,
            uses_alt: false,
        }
    }

    fn limit(n: u32) -> Vec<u8> {
        [vec![2], n.to_le_bytes().to_vec()].concat()
    }

    fn price(n: u64) -> Vec<u8> {
        [vec![3], n.to_le_bytes().to_vec()].concat()
    }

    #[test]
    fn default_counts_builtins_and_programs() {
        let b = compute_budget(&tx(vec![
            ("11111111111111111111111111111111", vec![]),
            ("SomeProgram", vec![]),
            ("OtherProgram", vec![]),
        ]));
        assert_eq!(b.cu_requested, 3_000 + 2 * 200_000);
        assert_eq!(b.priority_fee, 0);
    }

    #[test]
    fn default_is_capped() {
        let b = compute_budget(&tx((0..10).map(|_| ("P", vec![])).collect()));
        assert_eq!(b.cu_requested, MAX_COMPUTE_UNIT_LIMIT);
    }

    #[test]
    fn explicit_limit_and_price_round_up() {
        let b = compute_budget(&tx(vec![
            (COMPUTE_BUDGET_PROGRAM, limit(150_001)),
            (COMPUTE_BUDGET_PROGRAM, price(3)),
            ("P", vec![]),
        ]));
        assert_eq!(b.cu_requested, 150_001);
        assert_eq!(b.cu_price, 3);
        // 450_003 micro-lamports -> 0.450003 lamports -> rounds up to 1.
        assert_eq!(b.priority_fee, 1);
    }

    #[test]
    fn short_data_is_ignored() {
        let b = compute_budget(&tx(vec![
            (COMPUTE_BUDGET_PROGRAM, vec![2, 1]),
            ("P", vec![]),
        ]));
        assert_eq!(b.cu_requested, 3_000 + 200_000);
    }
}
