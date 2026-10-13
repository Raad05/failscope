//! Routing and validation, without a database: bad requests must be
//! rejected before any query runs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use failscope_api::http::router;
use failscope_store::{
    AlertEvent, AlertMetric, AlertState, AlertStore, Bucket, CoveragePart, ErrorCount,
    FailureFilter, FailureRow, NewAlertEvent, Page, Queries, Result, SignerFilter, SignerRow,
    SignerSummary, TimeBucket, TopProgram,
};
use http_body_util::BodyExt;
use std::collections::HashMap;
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
    async fn top_signers(&self, _: &SignerFilter, _: Page) -> Result<Vec<SignerRow>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
    }
    async fn signer_summary(&self, _: u32, _: Option<&str>) -> Result<SignerSummary> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(SignerSummary {
            failures: 0,
            affected_signers: 0,
            likely_human_signers: 0,
            unclear_signers: 0,
            likely_bot_signers: 0,
            unscored_signers: 0,
        })
    }
}

impl AlertStore for NoQueries {
    async fn per_minute(
        &self,
        _: AlertMetric,
        _: u32,
        _: &[String],
    ) -> Result<HashMap<String, Vec<u64>>> {
        unreachable!("the API never evaluates alerts")
    }
    async fn alert_states(&self, _: AlertMetric) -> Result<HashMap<String, AlertState>> {
        unreachable!("the API never evaluates alerts")
    }
    async fn record_alert(&self, _: &NewAlertEvent) -> Result<()> {
        unreachable!("the API never records alerts")
    }
    async fn alert_events(&self, _: u32, _: u32) -> Result<Vec<AlertEvent>> {
        assert!(
            self.allow,
            "query ran for a request that should have been rejected"
        );
        Ok(vec![])
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
        "/api/alerts?hours=0",
        "/api/alerts?offset=5",
        "/api/failures?bot_class=robot",
        "/api/failures?signer=not-base58!",
        "/api/signers/top?bot_class=bot",
        "/api/signers/top?hours=0",
        "/api/signers/top?program=short",
        "/api/signers/summary?hours=721",
        "/api/signers/summary?program=0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl",
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
        "/api/alerts?hours=48&limit=5",
        "/api/failures?bot_class=likely_human&signer=Dx7RPXNY6qwLY7352Wh5NyatNTSwssmcFiN4Bphs14g2",
        "/api/signers/top?hours=48&limit=5&offset=5&bot_class=likely_bot",
        "/api/signers/summary?hours=48",
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

#[tokio::test]
async fn dashboard_is_served_with_a_strict_policy() {
    for (uri, content_type) in [
        ("/", "text/html"),
        ("/assets/dashboard.css", "text/css"),
        ("/assets/dashboard.js", "text/javascript"),
    ] {
        let response = router(Arc::new(NoQueries { allow: false }))
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let headers = response.headers();
        assert!(
            headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with(content_type),
            "{uri}"
        );
        let csp = headers["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("script-src 'self'") && !csp.contains("unsafe-inline"),
            "{uri}: {csp}"
        );
        assert_eq!(headers["x-content-type-options"], "nosniff");
    }

    // The page must not rely on inline code the policy would block.
    let html = include_str!("../assets/index.html");
    assert!(!html.contains("<script>") && !html.contains(" style=") && !html.contains("onclick"));
    // Untrusted strings only through textContent.
    let js = include_str!("../assets/dashboard.js");
    assert!(!js.contains("innerHTML") && !js.contains("insertAdjacentHTML"));
}
