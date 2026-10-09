//! The API against a real Postgres, seeded with the 12 decoded fixtures.
//!
//! Needs `DATABASE_URL` (any database on the server; a fresh database is
//! created per run). Skips without it unless `FAILSCOPE_REQUIRE_DB=1`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use failscope_api::http::router;
use failscope_decoder::{decode, from_rpc_json, IdlErrorTable};
use failscope_store::{PgStore, Store};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const FAIL_TARGET: &str = "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey";
const JUPITER: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Creates a fresh database next to `DATABASE_URL` and connects to it.
async fn fresh_store() -> Option<(PgStore, PgStore, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) if std::env::var("FAILSCOPE_REQUIRE_DB").as_deref() == Ok("1") => {
            panic!("FAILSCOPE_REQUIRE_DB=1 but DATABASE_URL is not set")
        }
        Err(_) => {
            eprintln!("DATABASE_URL not set; skipping");
            return None;
        }
    };
    let admin = PgStore::connect(&url).await.unwrap();
    // Leftovers from runs that crashed before cleanup.
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE datname LIKE 'failscope_api_test_%'",
    )
    .fetch_all(admin.pool())
    .await
    .unwrap();
    for db in stale {
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
            .execute(admin.pool())
            .await;
    }
    let name = format!(
        "failscope_api_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let (base, _) = url.rsplit_once('/').unwrap();
    let store = PgStore::connect(&format!("{base}/{name}")).await.unwrap();
    Some((admin, store, name))
}

/// Decodes every fixture and stores it, spread over the last ~80 minutes,
/// plus one fail_target failure from two days ago (outside default windows).
async fn seed(store: &PgStore) -> Vec<String> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut idls = IdlErrorTable::new();
    for entry in fs::read_dir(fixtures.join("idls")).unwrap() {
        let path = entry.unwrap().path();
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        idls.add_idl(path.file_stem().unwrap().to_str().unwrap(), &json)
            .unwrap();
    }
    let mut dirs: Vec<_> = fs::read_dir(fixtures.join("txs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    let mut signatures = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        let json: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("tx.json")).unwrap()).unwrap();
        let mut failure = decode(&from_rpc_json(&json).unwrap(), &idls);
        failure.block_time = Some(now() - 60 - 7 * 60 * i as i64);
        assert!(store.insert_failure(&failure).await.unwrap());
        signatures.push(failure.signature.clone());
    }
    let mut old = decode(
        &from_rpc_json(
            &serde_json::from_str(&fs::read_to_string(dirs[0].join("tx.json")).unwrap()).unwrap(),
        )
        .unwrap(),
        &idls,
    );
    old.signature = "1".repeat(64);
    old.block_time = Some(now() - 48 * 3600);
    assert!(store.insert_failure(&old).await.unwrap());
    signatures
}

async fn get(store: &Arc<PgStore>, uri: &str) -> Value {
    let response = router(Arc::clone(store))
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{uri}");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn column(v: &Value, key: &str) -> Vec<Value> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[key].clone())
        .collect()
}

#[tokio::test]
async fn endpoints_against_seeded_database() {
    let Some((admin, store, name)) = fresh_store().await else {
        return;
    };
    let signatures = seed(&store).await;
    let store = Arc::new(store);

    // Top programs, last 24h: the 2-day-old row must not count.
    let top = get(&store, "/api/programs/top").await;
    let programs: Vec<&str> = top["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["program_id"].as_str().unwrap())
        .collect();
    assert_eq!(&programs[..2], &[FAIL_TARGET, JUPITER]);
    assert_eq!(column(&top, "failures")[..2], [6, 3].map(Value::from));
    assert_eq!(programs.len(), 5);
    let week = get(&store, "/api/programs/top?hours=168").await;
    assert_eq!(
        week["data"][0]["failures"], 7,
        "old row should count in a 7-day window"
    );

    // Pagination.
    let page2 = get(&store, "/api/programs/top?limit=2&offset=2").await;
    assert_eq!(page2["data"].as_array().unwrap().len(), 2);
    assert_eq!(
        page2["data"][0]["program_id"].as_str().unwrap(),
        programs[2]
    );

    // Errors of fail_target: six distinct errors, coded ones first.
    let errors = get(&store, &format!("/api/programs/{FAIL_TARGET}/errors")).await;
    let names: Vec<&str> = errors["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["error_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "ConstraintHasOne",
            "AccountNotSigner",
            "AlwaysFails",
            "CheckedOverflow",
            "ComputeUnitsExceeded",
            "ProgramPanicked"
        ]
    );

    // Coverage matches the fixture decode table.
    let coverage = get(&store, "/api/coverage").await;
    assert_eq!(coverage["total"], 12);
    let by_source: std::collections::HashMap<String, i64> = coverage["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["decode_source"].as_str().unwrap().to_string(),
                r["failures"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(by_source["anchor_log"], 5);
    assert_eq!(by_source["runtime"], 4);
    assert_eq!(
        (by_source["idl"], by_source["native"], by_source["unknown"]),
        (1, 1, 1)
    );

    // Time series: every failure lands in some bucket, empty buckets are zero.
    let ts = get(&store, "/api/failures/timeseries?hours=2&bucket=minute").await;
    let counts: Vec<i64> = ts["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["failures"].as_i64().unwrap())
        .collect();
    assert_eq!(counts.iter().sum::<i64>(), 12);
    assert!(counts.len() >= 120 && counts.contains(&0));
    let jup_ts = get(
        &store,
        &format!("/api/failures/timeseries?hours=2&program={JUPITER}"),
    )
    .await;
    assert_eq!(
        jup_ts["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["failures"].as_i64().unwrap())
            .sum::<i64>(),
        3
    );

    // Failure list and filters.
    let list = get(&store, "/api/failures?limit=5").await;
    assert_eq!(list["data"].as_array().unwrap().len(), 5);
    let jup = get(&store, &format!("/api/failures?program={JUPITER}")).await;
    assert_eq!(jup["data"].as_array().unwrap().len(), 3);
    let named = get(&store, "/api/failures?error_name=SlippageToleranceExceeded").await;
    assert_eq!(column(&named, "failing_program_id"), [Value::from(JUPITER)]);

    // Single failure.
    let one = get(&store, &format!("/api/failures/{}", signatures[0])).await;
    assert_eq!(one["signature"], signatures[0].as_str());
    assert!(one["block_time"].as_str().unwrap().ends_with('Z'));

    drop(store);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(admin.pool())
        .await
        .unwrap();
}
