//! Companion program for failscope: every instruction except `initialize`
//! fails on purpose, each in a different way, to produce real failed
//! transactions for the decoder's fixtures.

pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey");

#[program]
pub mod fail_target {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        instructions::initialize::handle_initialize(ctx)
    }

    pub fn fail_custom(ctx: Context<NoAccounts>) -> Result<()> {
        instructions::failures::handle_fail_custom(ctx)
    }

    pub fn fail_has_one(ctx: Context<FailHasOne>) -> Result<()> {
        instructions::failures::handle_fail_has_one(ctx)
    }

    pub fn fail_missing_signer(ctx: Context<FailMissingSigner>) -> Result<()> {
        instructions::failures::handle_fail_missing_signer(ctx)
    }

    pub fn fail_overflow_panic(ctx: Context<NoAccounts>, amount: u64) -> Result<()> {
        instructions::failures::handle_fail_overflow_panic(ctx, amount)
    }

    pub fn fail_overflow_checked(ctx: Context<NoAccounts>, amount: u64) -> Result<()> {
        instructions::failures::handle_fail_overflow_checked(ctx, amount)
    }

    pub fn fail_compute(ctx: Context<NoAccounts>) -> Result<()> {
        instructions::failures::handle_fail_compute(ctx)
    }

    pub fn fail_cpi_system(ctx: Context<FailCpiSystem>) -> Result<()> {
        instructions::failures::handle_fail_cpi_system(ctx)
    }

    pub fn fail_nested_cpi(ctx: Context<FailNestedCpi>) -> Result<()> {
        instructions::failures::handle_fail_nested_cpi(ctx)
    }
}
