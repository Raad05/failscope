#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use failscope_decoder::{from_rpc_json, TxInput};
use failscope_idl::{RpcAccount, StaticAccounts};

pub const FAIL_TARGET: &str = "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey";
pub const FAIL_CALLEE: &str = "BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb";
pub const JUPITER: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
/// Marinade: legacy location, legacy (pre-0.30) IDL format.
pub const MARINADE: &str = "MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD";
/// Mainnet program with no IDL in either location.
pub const NO_IDL: &str = "HBVw6bZtcCaezhcBrmfyXBSBRWCdv72271xQ4GPvms2z";

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Recorded on-chain accounts from fixtures/idl_accounts (null = no account).
pub fn recorded_accounts() -> StaticAccounts {
    let mut accounts = StaticAccounts::default();
    for entry in fs::read_dir(fixtures_dir().join("idl_accounts")).unwrap() {
        let file = read_json(&entry.unwrap().path());
        if file["value"].is_null() {
            continue;
        }
        let account: RpcAccount = serde_json::from_value(file["value"].clone()).unwrap();
        accounts.accounts.insert(
            file["address"].as_str().unwrap().to_string(),
            account.decode().unwrap(),
        );
    }
    accounts
}

pub fn fixture_tx(name: &str) -> TxInput {
    from_rpc_json(&read_json(
        &fixtures_dir().join("txs").join(name).join("tx.json"),
    ))
    .unwrap()
}

pub fn without_anchor_logs(mut tx: TxInput) -> TxInput {
    tx.logs = tx.logs.map(|l| {
        l.into_iter()
            .filter(|l| !l.contains("AnchorError"))
            .collect()
    });
    tx
}
