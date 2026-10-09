//! Captures failed-transaction updates as fixtures: for each one, the raw
//! protobuf `SubscribeUpdate` and the same transaction from JSON-RPC.
//!
//!     cargo run -p failscope-ingest --example capture -- <count> [grpc] [rpc]
//!
//! Writes fixtures/grpc/<signature>.pb and <signature>.rpc.json.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterTransactions,
};
use yellowstone_grpc_proto::prost::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().map_or(Ok(8), |c| c.parse())?;
    let grpc = args
        .next()
        .unwrap_or_else(|| "http://127.0.0.1:10000".to_string());
    let rpc = args
        .next()
        .unwrap_or_else(|| "http://127.0.0.1:8899".to_string());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/grpc");
    std::fs::create_dir_all(&out)?;

    let mut client = GeyserGrpcClient::build_from_shared(grpc)?.connect().await?;
    let request = SubscribeRequest {
        transactions: HashMap::from([(
            "failed".to_string(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(true),
                ..Default::default()
            },
        )]),
        commitment: Some(CommitmentLevel::Confirmed as i32),
        ..Default::default()
    };
    let (_sink, mut stream) = client.subscribe_with_request(Some(request)).await?;
    println!("waiting for {count} failed transactions...");

    let http = reqwest::Client::new();
    let mut captured = 0;
    while let Some(update) = stream.next().await {
        let update = update?;
        let Some(UpdateOneof::Transaction(t)) = &update.update_oneof else {
            continue;
        };
        let Some(info) = &t.transaction else { continue };
        let signature = bs58::encode(&info.signature).into_string();
        std::fs::write(out.join(format!("{signature}.pb")), update.encode_to_vec())?;

        // Confirmed on the stream means the RPC has it too, give or take a moment.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "getTransaction",
            "params": [signature, {"encoding": "json", "commitment": "confirmed",
                                   "maxSupportedTransactionVersion": 0}],
        });
        let response: serde_json::Value = http.post(&rpc).json(&body).send().await?.json().await?;
        let json = &response["result"];
        std::fs::write(
            out.join(format!("{signature}.rpc.json")),
            serde_json::to_string_pretty(json)? + "\n",
        )?;
        captured += 1;
        println!("{captured}/{count} {signature}");
        if captured == count {
            break;
        }
    }
    Ok(())
}
