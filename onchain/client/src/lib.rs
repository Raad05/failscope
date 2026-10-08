//! One builder per failure mode of `fail_target`. Used by the LiteSVM tests
//! and by the `send_failures` devnet sender, so both run the same transactions.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// CU limit for the compute-exhaustion case: low, so the loop dies fast.
pub const COMPUTE_CASE_CU_LIMIT: u32 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Custom,
    HasOne,
    MissingSigner,
    OverflowPanic,
    OverflowChecked,
    Compute,
    CpiSystem,
    NestedCpi,
}

impl Case {
    pub const ALL: [Case; 8] = [
        Case::Custom,
        Case::HasOne,
        Case::MissingSigner,
        Case::OverflowPanic,
        Case::OverflowChecked,
        Case::Compute,
        Case::CpiSystem,
        Case::NestedCpi,
    ];

    /// Stable name, used for fixture file names.
    pub fn slug(self) -> &'static str {
        match self {
            Case::Custom => "custom_error",
            Case::HasOne => "has_one_violation",
            Case::MissingSigner => "missing_signer",
            Case::OverflowPanic => "overflow_panic",
            Case::OverflowChecked => "overflow_checked",
            Case::Compute => "compute_exhausted",
            Case::CpiSystem => "cpi_system_transfer",
            Case::NestedCpi => "nested_cpi_callee",
        }
    }
}

/// A failing transaction's instructions plus any signers besides the payer.
pub struct Built {
    pub instructions: Vec<Instruction>,
    pub extra_signers: Vec<Keypair>,
}

pub fn counter_pda() -> Pubkey {
    Pubkey::find_program_address(&[fail_target::COUNTER_SEED], &fail_target::ID).0
}

/// Creates the counter PDA with `payer` as authority. Needed once, before
/// `Case::HasOne`.
pub fn initialize_ix(payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: fail_target::ID,
        accounts: fail_target::accounts::Initialize {
            payer: *payer,
            counter: counter_pda(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: fail_target::instruction::Initialize {}.data(),
    }
}

fn target_ix(accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: fail_target::ID,
        accounts,
        data,
    }
}

fn payer_only(payer: &Pubkey) -> Vec<AccountMeta> {
    fail_target::accounts::NoAccounts { payer: *payer }.to_account_metas(None)
}

pub fn build(case: Case, payer: &Pubkey) -> Built {
    let mut extra_signers = Vec::new();
    let instructions = match case {
        Case::Custom => vec![target_ix(
            payer_only(payer),
            fail_target::instruction::FailCustom {}.data(),
        )],
        Case::HasOne => {
            // Signs correctly, but is not the counter's authority.
            let impostor = Keypair::new();
            let ix = target_ix(
                fail_target::accounts::FailHasOne {
                    counter: counter_pda(),
                    authority: impostor.pubkey(),
                }
                .to_account_metas(None),
                fail_target::instruction::FailHasOne {}.data(),
            );
            extra_signers.push(impostor);
            vec![ix]
        }
        Case::MissingSigner => {
            // The program requires a signer; the message marks it as not
            // signing, so the tx is valid and lands, then fails validation.
            let mut accounts = fail_target::accounts::FailMissingSigner {
                authority: Keypair::new().pubkey(),
            }
            .to_account_metas(None);
            for meta in &mut accounts {
                meta.is_signer = false;
            }
            vec![target_ix(
                accounts,
                fail_target::instruction::FailMissingSigner {}.data(),
            )]
        }
        Case::OverflowPanic => vec![target_ix(
            payer_only(payer),
            fail_target::instruction::FailOverflowPanic { amount: 1 }.data(),
        )],
        Case::OverflowChecked => vec![target_ix(
            payer_only(payer),
            fail_target::instruction::FailOverflowChecked { amount: 1 }.data(),
        )],
        Case::Compute => vec![
            ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_CASE_CU_LIMIT),
            target_ix(
                payer_only(payer),
                fail_target::instruction::FailCompute {}.data(),
            ),
        ],
        Case::CpiSystem => vec![target_ix(
            fail_target::accounts::FailCpiSystem {
                payer: *payer,
                recipient: Keypair::new().pubkey(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            fail_target::instruction::FailCpiSystem {}.data(),
        )],
        Case::NestedCpi => vec![target_ix(
            fail_target::accounts::FailNestedCpi {
                payer: *payer,
                callee_program: fail_callee::ID,
            }
            .to_account_metas(None),
            fail_target::instruction::FailNestedCpi {}.data(),
        )],
    };
    Built {
        instructions,
        extra_signers,
    }
}
