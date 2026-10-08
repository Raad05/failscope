//! Error codes of native and SPL programs, keyed by program id, plus Anchor's
//! framework errors. Tables live in the generated `native_tables.rs`.

use crate::native_tables::{ANCHOR_FRAMEWORK, ASSOCIATED_TOKEN, SYSTEM, TOKEN, TOKEN_2022};

pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PE9qPdWLZDGkNvEQ";
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// Programs the runtime treats as builtins when computing the default CU
/// limit (agave 3.1 `solana-builtins-default-costs`; none are migrating).
pub const BUILTIN_PROGRAMS: [&str; 9] = [
    "Vote111111111111111111111111111111111111111",
    SYSTEM_PROGRAM,
    COMPUTE_BUDGET_PROGRAM,
    "BPFLoaderUpgradeab1e11111111111111111111111",
    "BPFLoader1111111111111111111111111111111111",
    "BPFLoader2111111111111111111111111111111111",
    "LoaderV411111111111111111111111111111111111",
    "KeccakSecp256k11111111111111111111111111111",
    "Ed25519SigVerify111111111111111111111111111",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownError {
    pub name: &'static str,
    pub message: &'static str,
}

fn find(table: &'static [(u32, &'static str, &'static str)], code: u32) -> Option<KnownError> {
    table
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, name, message)| KnownError { name, message })
}

/// Error `code` returned by a native or SPL program.
pub fn native_error(program_id: &str, code: u32) -> Option<KnownError> {
    let table = match program_id {
        SYSTEM_PROGRAM => SYSTEM,
        TOKEN_PROGRAM => TOKEN,
        TOKEN_2022_PROGRAM => TOKEN_2022,
        ASSOCIATED_TOKEN_PROGRAM => ASSOCIATED_TOKEN,
        _ => return None,
    };
    find(table, code)
}

/// Anchor framework error (codes below 6000). Only meaningful for programs
/// known to be built with Anchor.
pub fn anchor_framework_error(code: u32) -> Option<KnownError> {
    find(ANCHOR_FRAMEWORK, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_code_differs_by_program() {
        assert_eq!(
            native_error(SYSTEM_PROGRAM, 1).unwrap().name,
            "ResultWithNegativeLamports"
        );
        assert_eq!(
            native_error(TOKEN_PROGRAM, 1).unwrap().name,
            "InsufficientFunds"
        );
        assert_eq!(native_error("SomethingElse", 1), None);
    }

    #[test]
    fn token_2022_extends_token() {
        for code in 0..20 {
            assert_eq!(
                native_error(TOKEN_PROGRAM, code).map(|e| e.name),
                native_error(TOKEN_2022_PROGRAM, code).map(|e| e.name),
                "code {code}"
            );
        }
        assert!(native_error(TOKEN_PROGRAM, 20).is_none());
        assert!(native_error(TOKEN_2022_PROGRAM, 20).is_some());
    }

    #[test]
    fn anchor_framework_codes() {
        assert_eq!(
            anchor_framework_error(2001).unwrap().name,
            "ConstraintHasOne"
        );
        assert_eq!(
            anchor_framework_error(3010).unwrap().name,
            "AccountNotSigner"
        );
        assert_eq!(anchor_framework_error(6000), None);
    }
}
