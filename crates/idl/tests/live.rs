//! Same checks against real RPCs. Network-dependent, so ignored by default:
//!
//!     cargo test -p failscope-idl --test live -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use failscope_idl::{fetch_idl, IdlLocation, RpcAccountSource};

const DEVNET: &str = "https://api.devnet.solana.com";
const MAINNET: &str = "https://api.mainnet-beta.solana.com";

#[tokio::test]
#[ignore = "needs network"]
async fn devnet_program_metadata() {
    let idl = fetch_idl(&RpcAccountSource::new(DEVNET), FAIL_TARGET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idl.location, IdlLocation::ProgramMetadata);
    assert_eq!(idl.json["errors"][0]["name"], "AlwaysFails");
}

#[tokio::test]
#[ignore = "needs network"]
async fn mainnet_legacy_and_missing() {
    let rpc = RpcAccountSource::new(MAINNET);
    let jup = fetch_idl(&rpc, JUPITER).await.unwrap().unwrap();
    assert_eq!(jup.location, IdlLocation::Legacy);
    assert_eq!(fetch_idl(&rpc, NO_IDL).await.unwrap(), None);
}
