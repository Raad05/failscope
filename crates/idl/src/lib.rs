//! Fetches, parses and caches on-chain Anchor IDLs.
//!
//! An IDL can live in two places (see `address`): the legacy Anchor IDL
//! account (Anchor < 1.0) or a Program Metadata account (Anchor >= 1.0).
//! Either location can hold either IDL format (legacy or 0.30+ spec):
//! Jupiter v6 stores a spec-format IDL in the legacy account.

mod account;
mod address;
mod cache;
mod fetch;
mod source;

pub use account::{decode_legacy_account, decode_metadata_account, MAX_IDL_BYTES};
pub use address::{
    legacy_idl_address, metadata_idl_address, parse_address, METADATA_IDL_SEED,
    PROGRAM_METADATA_PROGRAM,
};
pub use cache::{CacheEntry, CacheTtl, FetchStatus, IdlCache};
pub use fetch::{detect_format, fetch_idl, Idl, IdlFormat, IdlLocation};
pub use source::{AccountData, AccountSource, RpcAccount, RpcAccountSource, StaticAccounts};

#[derive(Debug, thiserror::Error)]
pub enum IdlError {
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("malformed IDL account: {0}")]
    Malformed(&'static str),
    #[error("unsupported IDL account: {0}")]
    Unsupported(String),
    #[error("IDL is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("RPC request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("RPC error: {0}")]
    Rpc(String),
}
