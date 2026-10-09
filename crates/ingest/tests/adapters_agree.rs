//! The Yellowstone adapter and the RPC adapter must produce the same decoder
//! input (and so the same decode) for the same transaction. Fixtures were
//! captured from a local validator with the Yellowstone plugin
//! (examples/capture.rs).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use failscope_decoder::{decode, from_rpc_json, NoIdls};
use failscope_ingest::tx_input;
use failscope_ingest::yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, SubscribeUpdate,
};
use failscope_ingest::yellowstone_grpc_proto::prost::Message;

#[test]
fn grpc_and_rpc_adapters_agree() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/grpc");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("pb") {
            continue;
        }
        let update = SubscribeUpdate::decode(fs::read(&path).unwrap().as_slice()).unwrap();
        let Some(UpdateOneof::Transaction(t)) = update.update_oneof else {
            panic!("{}: not a transaction update", path.display())
        };
        let from_grpc = tx_input(t.slot, t.transaction.as_ref().unwrap())
            .unwrap()
            .unwrap();

        let rpc_path = path.with_extension("rpc.json");
        let rpc_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&rpc_path).unwrap()).unwrap();
        let mut from_rpc = from_rpc_json(&rpc_json).unwrap();

        // Transaction updates carry no block time; it is joined from block meta.
        assert_eq!(from_grpc.block_time, None);
        from_rpc.block_time = None;
        assert_eq!(from_grpc, from_rpc, "{}", path.display());
        assert_eq!(decode(&from_grpc, &NoIdls), decode(&from_rpc, &NoIdls));
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} captured fixtures");
}
