//! Second program for the nested-CPI failure case. `fail_target` calls
//! `fail_inner`, which always fails with this program's own error 6000.
//! The code collides on purpose with `fail_target`'s 6000, so decoding has to
//! be keyed by (program_id, code).

use anchor_lang::prelude::*;

declare_id!("BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb");

#[program]
pub mod fail_callee {
    use super::*;

    pub fn fail_inner(_ctx: Context<FailInner>) -> Result<()> {
        err!(CalleeError::CalleeAlwaysFails)
    }
}

#[derive(Accounts)]
pub struct FailInner<'info> {
    pub caller: Signer<'info>,
}

#[error_code]
pub enum CalleeError {
    #[msg("fail_callee always fails when invoked")]
    CalleeAlwaysFails,
}
