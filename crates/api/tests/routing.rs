//! Routing and validation, without a database: bad requests must be
//! rejected before any query runs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use failscope_api::http::router;
use failscope_store::{
    Bucket, CoveragePart, ErrorCount, FailureFilter, FailureRow, Page, Queries, Result, TimeBucket,
    TopProgram,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Panics if a query runs: every request in these tests must be answered
/// before reaching the store, or return empty data.
struct NoQueries {
    allow: bool,
}

impl Queries for NoQueries {
    async fn top_programs(&self, _: u32, _: Page) -> Result<Vec<TopProgram>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn program_errors(&self, _: &str, _: u32, _: Page) -> Result<Vec<ErrorCount>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn failures_over_time(
        &self,
        _: u32,
        _: TimeBucket,
        _: Option<&str>,
    ) -> Result<Vec<Bucket>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn coverage(&self, _: u32) -> Result<Vec<CoveragePart>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn failures(&self, _: &FailureFilter, _: Page) -> Result<Vec<FailureRow>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn failure(&self, _: &str) -> Result<Option<FailureRow>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(None)
    }
}

async fn get(allow: bool, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = router(Arc::new(NoQueries { allow }))
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn rejects_bad_parameters_before_querying() {
    for uri in [
        "/api/programs/top?hours=0",
        "/api/programs/top?hours=721",
        "/api/programs/top?limit=0",
        "/api/programs/top?limit=501",
        "/api/programs/top?offset=1000001",
        "/api/programs/not-a-key!/errors",
        "/api/programs/short/errors",
        "/api/failures?program=0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl",
        "/api/failures/timeseries?bucket=minute&hours=25",
        "/api/failures/timeseries?bucket=week",
        "/api/programs/top?limit=abc",
        "/api/failures/tooshort",
        "/api/coverage?hours=9999",
    ] {
        let (status, body) = get(false, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert!(body["error"].is_string(), "{uri}: not a JSON error: {body}");
    }
}

#[tokio::test]
async fn defaults_and_envelope() {
    let (status, body) = get(true, "/api/programs/top").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["window_hours"], 24);
    assert_eq!(body["page"]["limit"], 50);
    assert_eq!(body["page"]["offset"], 0);
    assert_eq!(body["data"], serde_json::json!([]));

    // Numeric params on every list endpoint (a flattened struct once broke these).
    for uri in [
        "/api/programs/top?hours=48&limit=5&offset=5",
        "/api/programs/6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey/errors?hours=48&limit=5&offset=5",
        "/api/failures?hours=48&limit=5&offset=5",
        "/api/failures/timeseries?hours=48&bucket=day",
        "/api/coverage?hours=48",
    ] {
        let (status, body) = get(true, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }

    let (_, ts) = get(true, "/api/failures/timeseries").await;
    assert_eq!(ts["bucket"], "hour");

    let (status, _) = get(true, "/health").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_signature_is_404() {
    let sig =
        "5WAFxXMnhR22RcddzQF8BmJ234WvB2V7RsWG9nSWpq2wsVz3h7No4ZLpZjFzetTFcKJ9f6MLtSAvqnqAboQ5s7w3";
    let (status, body) = get(true, &format!("/api/failures/{sig}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains(sig));
}
