//! Instructions that fail on purpose, each in a distinct way. See
//! `fixtures/README.md` for how each one shows up on chain.

use std::hint::black_box;

use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
use fail_callee::program::FailCallee;

use crate::{constants::*, error::FailTargetError, state::Counter};

/// Context for instructions that need no state. Anchor's CPI codegen needs
/// at least one account, so the fee payer is passed.
#[derive(Accounts)]
pub struct NoAccounts<'info> {
    pub payer: Signer<'info>,
}

/// User error via `require!` (code 6000).
pub fn handle_fail_custom(_ctx: Context<NoAccounts>) -> Result<()> {
    require!(black_box(false), FailTargetError::AlwaysFails);
    Ok(())
}

#[derive(Accounts)]
pub struct FailHasOne<'info> {
    #[account(seeds = [COUNTER_SEED], bump, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

/// Fails in account validation (ConstraintHasOne, 2001) when `authority`
/// is not the counter's authority.
pub fn handle_fail_has_one(_ctx: Context<FailHasOne>) -> Result<()> {
    Ok(())
}

#[derive(Accounts)]
pub struct FailMissingSigner<'info> {
    pub authority: Signer<'info>,
}

/// Fails in account validation (AccountNotSigner, 3010) when the client
/// passes `authority` with `is_signer = false`.
pub fn handle_fail_missing_signer(_ctx: Context<FailMissingSigner>) -> Result<()> {
    Ok(())
}

/// Unchecked add with `overflow-checks = true`: the program panics.
pub fn handle_fail_overflow_panic(_ctx: Context<NoAccounts>, amount: u64) -> Result<()> {
    let sum = black_box(u64::MAX) + amount;
    msg!("unreachable for amount > 0: {}", sum);
    Ok(())
}

/// Checked add returning a user error (code 6001).
pub fn handle_fail_overflow_checked(_ctx: Context<NoAccounts>, amount: u64) -> Result<()> {
    let sum = black_box(u64::MAX)
        .checked_add(amount)
        .ok_or(FailTargetError::CheckedOverflow)?;
    msg!("unreachable for amount > 0: {}", sum);
    Ok(())
}

/// Spins until the compute meter runs out.
pub fn handle_fail_compute(_ctx: Context<NoAccounts>) -> Result<()> {
    let mut i: u64 = 0;
    loop {
        i = black_box(i.wrapping_add(1));
        if i == 0 {
            return Ok(());
        }
    }
}

#[derive(Accounts)]
pub struct FailCpiSystem<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: any account; the transfer fails before it is credited.
    #[account(mut)]
    pub recipient: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// CPI into the System program asking for more lamports than the payer has.
/// The failure belongs to the System program, not to this one.
pub fn handle_fail_cpi_system(ctx: Context<FailCpiSystem>) -> Result<()> {
    let accounts = Transfer {
        from: ctx.accounts.payer.to_account_info(),
        to: ctx.accounts.recipient.to_account_info(),
    };
    system_program::transfer(CpiContext::new(system_program::ID, accounts), u64::MAX)
}

#[derive(Accounts)]
pub struct FailNestedCpi<'info> {
    pub payer: Signer<'info>,
    pub callee_program: Program<'info, FailCallee>,
}

/// CPI into `fail_callee`, which fails with its own error 6000.
pub fn handle_fail_nested_cpi(ctx: Context<FailNestedCpi>) -> Result<()> {
    fail_callee::cpi::fail_inner(CpiContext::new(
        fail_callee::ID,
        fail_callee::cpi::accounts::FailInner {
            caller: ctx.accounts.payer.to_account_info(),
        },
    ))
}
