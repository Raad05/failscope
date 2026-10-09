//! JSON API. Every list endpoint takes `hours` (window, default 24, max 720)
//! plus `limit` (default 50, max 500) and `offset`, and answers
//! `{"data": [...], "page": {"limit", "offset"}, "window_hours"}`.
//! Errors are `{"error": "..."}` with a 4xx/5xx status.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use failscope_store::{FailureFilter, Page, Queries, TimeBucket, MAX_LIMIT};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

pub const DEFAULT_HOURS: u32 = 24;
pub const MAX_HOURS: u32 = 720;
pub const DEFAULT_LIMIT: u32 = 50;
const MAX_OFFSET: u32 = 1_000_000;

pub fn router<Q: Queries + 'static>(queries: Arc<Q>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/programs/top", get(top_programs::<Q>))
        .route(
            "/api/programs/{program_id}/errors",
            get(program_errors::<Q>),
        )
        .route("/api/failures", get(failures::<Q>))
        .route("/api/failures/timeseries", get(timeseries::<Q>))
        .route("/api/failures/{signature}", get(failure::<Q>))
        .route("/api/coverage", get(coverage::<Q>))
        .layer(TraceLayer::new_for_http())
        .with_state(queries)
}

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<failscope_store::StoreError> for ApiError {
    fn from(e: failscope_store::StoreError) -> Self {
        // Details go to the log, not to the client.
        tracing::error!(error = %e, "query failed");
        ApiError::Internal
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// ---------------------------------------------------------------- params

/// `Query` whose rejections (unparseable values) use the JSON error shape.
pub struct ApiQuery<T>(pub T);

impl<T, St> axum::extract::FromRequestParts<St> for ApiQuery<T>
where
    T: serde::de::DeserializeOwned,
    St: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &St,
    ) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(v)| ApiQuery(v))
            .map_err(|e| ApiError::BadRequest(e.body_text()))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct WindowParams {
    pub hours: Option<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl WindowParams {
    fn hours(&self) -> Result<u32, ApiError> {
        match self.hours.unwrap_or(DEFAULT_HOURS) {
            h @ 1..=MAX_HOURS => Ok(h),
            h => Err(ApiError::BadRequest(format!(
                "hours must be 1..={MAX_HOURS}, got {h}"
            ))),
        }
    }

    fn page(&self) -> Result<Page, ApiError> {
        let limit = match self.limit.unwrap_or(DEFAULT_LIMIT) {
            l @ 1..=MAX_LIMIT => l,
            l => {
                return Err(ApiError::BadRequest(format!(
                    "limit must be 1..={MAX_LIMIT}, got {l}"
                )))
            }
        };
        let offset = self.offset.unwrap_or(0);
        if offset > MAX_OFFSET {
            return Err(ApiError::BadRequest(format!(
                "offset must be <= {MAX_OFFSET}"
            )));
        }
        Ok(Page { limit, offset })
    }
}

#[derive(Debug, Serialize)]
pub struct Listing<T> {
    pub data: Vec<T>,
    pub page: Page,
    pub window_hours: u32,
}

/// Base58 of a plausible length; rejects junk before it reaches a query.
fn check_base58(what: &str, s: &str, len: std::ops::RangeInclusive<usize>) -> Result<(), ApiError> {
    let ok = len.contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() && !matches!(b, b'0' | b'O' | b'I' | b'l'));
    if ok {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{what} is not a valid base58 {what}"
        )))
    }
}

fn check_program(s: &str) -> Result<(), ApiError> {
    check_base58("program id", s, 32..=44)
}

// ---------------------------------------------------------------- handlers

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn top_programs<Q: Queries>(
    State(q): State<Arc<Q>>,
    ApiQuery(p): ApiQuery<WindowParams>,
) -> ApiResult<Listing<failscope_store::TopProgram>> {
    let (hours, page) = (p.hours()?, p.page()?);
    Ok(Json(Listing {
        data: q.top_programs(hours, page).await?,
        page,
        window_hours: hours,
    }))
}

async fn program_errors<Q: Queries>(
    State(q): State<Arc<Q>>,
    Path(program_id): Path<String>,
    ApiQuery(p): ApiQuery<WindowParams>,
) -> ApiResult<Listing<failscope_store::ErrorCount>> {
    check_program(&program_id)?;
    let (hours, page) = (p.hours()?, p.page()?);
    Ok(Json(Listing {
        data: q.program_errors(&program_id, hours, page).await?,
        page,
        window_hours: hours,
    }))
}

// Fields listed out instead of `#[serde(flatten)] WindowParams`: flatten
// buffers query values as strings, and the numbers then fail to parse.
#[derive(Debug, Deserialize)]
pub struct FailureParams {
    pub hours: Option<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub program: Option<String>,
    pub error_name: Option<String>,
}

async fn failures<Q: Queries>(
    State(q): State<Arc<Q>>,
    ApiQuery(p): ApiQuery<FailureParams>,
) -> ApiResult<Listing<failscope_store::FailureRow>> {
    if let Some(program) = &p.program {
        check_program(program)?;
    }
    if p.error_name.as_ref().is_some_and(|n| n.len() > 128) {
        return Err(ApiError::BadRequest("error_name is too long".to_string()));
    }
    let window = WindowParams {
        hours: p.hours,
        limit: p.limit,
        offset: p.offset,
    };
    let (hours, page) = (window.hours()?, window.page()?);
    let filter = FailureFilter {
        hours,
        program_id: p.program,
        error_name: p.error_name,
    };
    Ok(Json(Listing {
        data: q.failures(&filter, page).await?,
        page,
        window_hours: hours,
    }))
}

async fn failure<Q: Queries>(
    State(q): State<Arc<Q>>,
    Path(signature): Path<String>,
) -> ApiResult<failscope_store::FailureRow> {
    check_base58("signature", &signature, 64..=88)?;
    q.failure(&signature)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no failed transaction {signature}")))
}

#[derive(Debug, Deserialize)]
pub struct TimeseriesParams {
    pub hours: Option<u32>,
    pub bucket: Option<TimeBucket>,
    pub program: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Timeseries {
    pub data: Vec<failscope_store::Bucket>,
    pub bucket: TimeBucket,
    pub window_hours: u32,
    pub program: Option<String>,
}

async fn timeseries<Q: Queries>(
    State(q): State<Arc<Q>>,
    ApiQuery(p): ApiQuery<TimeseriesParams>,
) -> ApiResult<Timeseries> {
    let hours = WindowParams {
        hours: p.hours,
        ..Default::default()
    }
    .hours()?;
    let bucket = p.bucket.unwrap_or(TimeBucket::Hour);
    // Keep charts to at most 1440 points.
    if bucket == TimeBucket::Minute && hours > 24 {
        return Err(ApiError::BadRequest(
            "bucket=minute allows at most hours=24".to_string(),
        ));
    }
    if let Some(program) = &p.program {
        check_program(program)?;
    }
    Ok(Json(Timeseries {
        data: q
            .failures_over_time(hours, bucket, p.program.as_deref())
            .await?,
        bucket,
        window_hours: hours,
        program: p.program,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CoverageParams {
    pub hours: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Coverage {
    pub data: Vec<failscope_store::CoveragePart>,
    pub total: i64,
    pub window_hours: u32,
}

async fn coverage<Q: Queries>(
    State(q): State<Arc<Q>>,
    ApiQuery(p): ApiQuery<CoverageParams>,
) -> ApiResult<Coverage> {
    let hours = WindowParams {
        hours: p.hours,
        ..Default::default()
    }
    .hours()?;
    let data = q.coverage(hours).await?;
    Ok(Json(Coverage {
        total: data.iter().map(|c| c.failures).sum(),
        data,
        window_hours: hours,
    }))
}
