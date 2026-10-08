use anchor_lang::prelude::*;

#[error_code]
pub enum FailTargetError {
    #[msg("This instruction always fails on purpose")]
    AlwaysFails,
    #[msg("Checked arithmetic overflowed")]
    CheckedOverflow,
}
