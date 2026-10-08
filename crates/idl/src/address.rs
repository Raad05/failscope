//! Where a program's IDL can live on chain. Both derivations are verified
//! against real accounts (see fixtures/idl_accounts).

use solana_address::Address;

use crate::IdlError;

/// solana-foundation/program-metadata. Anchor >= 1.0 writes IDLs here.
pub const PROGRAM_METADATA_PROGRAM: &str = "ProgM6JCCvbYkfKqJYHePx4xxSUSqJp7rh8Lyv7nk7S";

/// Seed of the canonical IDL metadata account: `idl`, zero-padded to 16 bytes.
pub const METADATA_IDL_SEED: [u8; 16] = *b"idl\0\0\0\0\0\0\0\0\0\0\0\0\0";

const LEGACY_IDL_SEED: &str = "anchor:idl";

pub fn parse_address(s: &str) -> Result<Address, IdlError> {
    s.parse()
        .map_err(|_| IdlError::InvalidAddress(s.to_string()))
}

/// Legacy Anchor IDL account (Anchor < 1.0):
/// `create_with_seed(find_program_address([], program), "anchor:idl", program)`.
/// Owned by the program itself.
pub fn legacy_idl_address(program: &Address) -> Result<Address, IdlError> {
    let (base, _) = Address::find_program_address(&[], program);
    Address::create_with_seed(&base, LEGACY_IDL_SEED, program)
        .map_err(|e| IdlError::InvalidAddress(format!("create_with_seed: {e}")))
}

/// Canonical Program Metadata account for the IDL (no authority in the seeds):
/// `find_program_address([program, "idl" padded to 16], PROGRAM_METADATA_PROGRAM)`.
pub fn metadata_idl_address(program: &Address) -> Result<Address, IdlError> {
    let pmp = parse_address(PROGRAM_METADATA_PROGRAM)?;
    let (address, _) = Address::find_program_address(&[program.as_ref(), &METADATA_IDL_SEED], &pmp);
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JUPITER: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

    #[test]
    fn jupiter_addresses_match_chain() {
        let p = parse_address(JUPITER).unwrap();
        assert_eq!(
            legacy_idl_address(&p).unwrap().to_string(),
            "C88XWfp26heEmDkmfSzeXP7Fd7GQJ2j9dDTUsyiZbUTa"
        );
        // The address Anchor CLI 1.2 queried for Jupiter.
        assert_eq!(
            metadata_idl_address(&p).unwrap().to_string(),
            "FDDfotwLyeLhUQ62ugzgTjwTvF3r64tPRVsKwsqRrbbC"
        );
    }

    #[test]
    fn fail_target_metadata_address_matches_devnet() {
        let p = parse_address("6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey").unwrap();
        assert_eq!(
            metadata_idl_address(&p).unwrap().to_string(),
            "HZbAvBW8W7gtrP7f5a9ucr3SirmfLoZWSRVVr5sseBE2"
        );
    }

    #[test]
    fn rejects_bad_addresses() {
        assert!(parse_address("not-a-key").is_err());
    }
}
