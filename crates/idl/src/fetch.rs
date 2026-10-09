//! Finds and decodes a program's IDL from either on-chain location.

use serde::Serialize;

use crate::account::{decode_legacy_account, decode_metadata_account};
use crate::address::{
    legacy_idl_address, metadata_idl_address, parse_address, PROGRAM_METADATA_PROGRAM,
};
use crate::source::AccountSource;
use crate::IdlError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdlLocation {
    /// Anchor < 1.0 `anchor:idl` account, owned by the program.
    Legacy,
    /// Program Metadata Program account (Anchor >= 1.0).
    ProgramMetadata,
}

impl IdlLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            IdlLocation::Legacy => "legacy",
            IdlLocation::ProgramMetadata => "program_metadata",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "legacy" => Some(IdlLocation::Legacy),
            "program_metadata" => Some(IdlLocation::ProgramMetadata),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "spec", rename_all = "snake_case")]
pub enum IdlFormat {
    /// Pre-0.30 IDL: top-level `name`/`version`, no `address`.
    Legacy,
    /// 0.30+ spec: top-level `address` and `metadata.spec`.
    Spec(String),
}

impl IdlFormat {
    /// `legacy` or `spec:<version>`.
    pub fn to_text(&self) -> String {
        match self {
            IdlFormat::Legacy => "legacy".to_string(),
            IdlFormat::Spec(v) => format!("spec:{v}"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.split_once(':') {
            None if s == "legacy" => Some(IdlFormat::Legacy),
            Some(("spec", v)) => Some(IdlFormat::Spec(v.to_string())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Idl {
    pub program_id: String,
    pub location: IdlLocation,
    pub format: IdlFormat,
    pub json: serde_json::Value,
}

pub fn detect_format(idl: &serde_json::Value) -> IdlFormat {
    let spec = idl
        .get("metadata")
        .and_then(|m| m.get("spec"))
        .and_then(|s| s.as_str());
    match (idl.get("address"), spec) {
        (Some(_), Some(spec)) => IdlFormat::Spec(spec.to_string()),
        _ => IdlFormat::Legacy,
    }
}

/// Looks in both locations with one RPC call. If both hold an IDL, the
/// Program Metadata one wins: Anchor >= 1.0 can only update that one, so the
/// legacy account may be stale. Returns `Ok(None)` if the program has no IDL.
pub async fn fetch_idl<S: AccountSource>(
    source: &S,
    program_id: &str,
) -> Result<Option<Idl>, IdlError> {
    let program = parse_address(program_id)?;
    let addresses = [
        metadata_idl_address(&program)?.to_string(),
        legacy_idl_address(&program)?.to_string(),
    ];
    let mut accounts = source.get_accounts(&addresses).await?.into_iter();
    let (metadata, legacy) = (accounts.next().flatten(), accounts.next().flatten());

    let mut last_error = None;
    let candidates = [
        (
            IdlLocation::ProgramMetadata,
            metadata,
            PROGRAM_METADATA_PROGRAM,
        ),
        (IdlLocation::Legacy, legacy, program_id),
    ];
    for (location, account, expected_owner) in candidates {
        let Some(account) = account else { continue };
        if account.owner != expected_owner {
            // Someone else's account at that address; not an IDL.
            continue;
        }
        let decoded = match location {
            IdlLocation::ProgramMetadata => decode_metadata_account(&account.data),
            IdlLocation::Legacy => decode_legacy_account(&account.data),
        };
        match decoded.and_then(|bytes| Ok(serde_json::from_slice(&bytes)?)) {
            Ok(json) => {
                return Ok(Some(Idl {
                    program_id: program_id.to_string(),
                    location,
                    format: detect_format(&json),
                    json,
                }))
            }
            Err(e) => last_error = Some(e),
        }
    }
    match last_error {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_round_trips() {
        for f in [IdlFormat::Legacy, IdlFormat::Spec("0.1.0".to_string())] {
            assert_eq!(IdlFormat::parse(&f.to_text()), Some(f));
        }
        for l in [IdlLocation::Legacy, IdlLocation::ProgramMetadata] {
            assert_eq!(IdlLocation::parse(l.as_str()), Some(l));
        }
    }

    #[test]
    fn formats() {
        assert_eq!(
            detect_format(&json!({"address": "x", "metadata": {"spec": "0.1.0"}})),
            IdlFormat::Spec("0.1.0".to_string())
        );
        assert_eq!(
            detect_format(&json!({"version": "0.1.0", "name": "x", "instructions": []})),
            IdlFormat::Legacy
        );
    }
}
