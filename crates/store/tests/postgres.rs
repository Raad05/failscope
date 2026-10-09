//! Runs the Postgres store against a real database.
//!
//! Needs `DATABASE_URL`. Without it the tests skip, unless
//! `FAILSCOPE_REQUIRE_DB=1` (set in CI), where a missing database fails.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{SystemTime, UNIX_EPOCH};

use failscope_decoder::{Confidence, DecodeSource, DecodedFailure};
use failscope_store::{PgStore, Store};

async fn store() -> Option<PgStore> {
    match std::env::var("DATABASE_URL") {
        Ok(url) => Some(PgStore::connect(&url).await.unwrap()),
        Err(_) if std::env::var("FAILSCOPE_REQUIRE_DB").as_deref() == Ok("1") => {
            panic!("FAILSCOPE_REQUIRE_DB=1 but DATABASE_URL is not set")
        }
        Err(_) => {
            eprintln!("DATABASE_URL not set; skipping Postgres tests");
            None
        }
    }
}

/// Unique per test run so tests don't see each other's rows.
fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}

fn failure(signature: &str, slot: u64) -> DecodedFailure {
    DecodedFailure {
        signature: signature.to_string(),
        slot,
        block_time: None,
        fee: 5000,
        priority_fee: 0,
        cu_requested: 200_000,
        cu_consumed: Some(1234),
        signer: Some("payer".to_string()),
        uses_alt: false,
        log_truncated: false,
        top_level_ix_index: Some(0),
        root_program_id: Some("root".to_string()),
        failing_program_id: Some("prog".to_string()),
        cpi_depth: Some(1),
        attribution_confidence: Confidence::High,
        error_kind: "custom".to_string(),
        error_code: Some(6000),
        error_name: Some("AlwaysFails".to_string()),
        error_message: Some("m".to_string()),
        error_account: None,
        decode_source: DecodeSource::AnchorLog,
        raw_err: serde_json::json!({"InstructionError": [0, {"Custom": 6000}]}),
    }
}

#[tokio::test]
async fn insert_is_idempotent_and_round_trips() {
    let Some(store) = store().await else { return };
    let sig = unique("sig");
    let slot = 4_000_000_000;
    assert!(store.insert_failure(&failure(&sig, slot)).await.unwrap());
    assert!(
        !store.insert_failure(&failure(&sig, slot)).await.unwrap(),
        "duplicate inserted"
    );

    let (source, confidence, code, raw): (String, String, Option<i64>, serde_json::Value) =
        sqlx::query_as(
            "SELECT decode_source, attribution_confidence, error_code, raw_err
             FROM failed_tx WHERE signature = $1",
        )
        .bind(&sig)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        (source.as_str(), confidence.as_str(), code),
        ("anchor_log", "high", Some(6000))
    );
    assert_eq!(raw["InstructionError"][1]["Custom"], 6000);
}

#[tokio::test]
async fn block_time_is_filled_once() {
    let Some(store) = store().await else { return };
    let slot = 4_100_000_000
        + (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros()
            % 1_000_000) as u64;
    store
        .insert_failure(&failure(&unique("a"), slot))
        .await
        .unwrap();
    store
        .insert_failure(&failure(&unique("b"), slot))
        .await
        .unwrap();
    assert_eq!(store.set_block_time(slot, 1_790_000_000).await.unwrap(), 2);
    assert_eq!(
        store.set_block_time(slot, 1).await.unwrap(),
        0,
        "overwrote a block time"
    );
}

#[tokio::test]
async fn cursor_never_moves_backwards() {
    let Some(store) = store().await else { return };
    let stream = unique("stream");
    assert_eq!(store.cursor(&stream).await.unwrap(), None);
    store.save_cursor(&stream, 100).await.unwrap();
    store.save_cursor(&stream, 90).await.unwrap();
    assert_eq!(store.cursor(&stream).await.unwrap(), Some(100));
    store.save_cursor(&stream, 120).await.unwrap();
    assert_eq!(store.cursor(&stream).await.unwrap(), Some(120));
}

#[tokio::test]
async fn gaps_are_recorded() {
    let Some(store) = store().await else { return };
    let stream = unique("gap");
    store.record_gap(&stream, 10, 20, "test").await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM ingest_gaps WHERE stream = $1")
        .bind(&stream)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn idl_cache_round_trips_and_keeps_age() {
    use failscope_store::IdlCacheRow;
    let Some(store) = store().await else { return };
    let found = IdlCacheRow {
        program_id: unique("prog"),
        fetch_status: "found".to_string(),
        location: Some("program_metadata".to_string()),
        idl_format: Some("spec:0.1.0".to_string()),
        idl_json: Some(serde_json::json!({"errors": [{"code": 6000, "name": "A"}]})),
        age_secs: 120,
    };
    let missing = IdlCacheRow {
        program_id: unique("none"),
        fetch_status: "missing".to_string(),
        location: None,
        idl_format: None,
        idl_json: None,
        age_secs: 0,
    };
    store.upsert_idl(&found).await.unwrap();
    store.upsert_idl(&missing).await.unwrap();
    // Upsert replaces.
    store
        .upsert_idl(&IdlCacheRow {
            age_secs: 300,
            ..found.clone()
        })
        .await
        .unwrap();

    let rows = store.load_idls().await.unwrap();
    let got = rows
        .iter()
        .find(|r| r.program_id == found.program_id)
        .unwrap();
    assert_eq!(got.idl_json, found.idl_json);
    assert_eq!(got.location, found.location);
    assert!((300..310).contains(&got.age_secs), "age {}", got.age_secs);
    let got_missing = rows
        .iter()
        .find(|r| r.program_id == missing.program_id)
        .unwrap();
    assert_eq!(got_missing.fetch_status, "missing");
    assert_eq!(got_missing.idl_json, None);
}
