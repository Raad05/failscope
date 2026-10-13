//! Read-side queries for the API. Time windows are "last N hours" over
//! `COALESCE(block_time, ingested_at)`, so rows still waiting for their block
//! time count too. Timestamps come back as RFC 3339 strings in UTC.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::{PgStore, Result};

/// Largest page any query returns.
pub const MAX_LIMIT: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Page {
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeBucket {
    Minute,
    Hour,
    Day,
}

impl TimeBucket {
    fn sql(self) -> &'static str {
        match self {
            TimeBucket::Minute => "minute",
            TimeBucket::Hour => "hour",
            TimeBucket::Day => "day",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct TopProgram {
    pub program_id: String,
    pub failures: i64,
    pub distinct_errors: i64,
    pub last_seen: Option<String>,
    /// Distinct signers with a failure in this program. Each signer is
    /// classed by its most bot-like failure in the window; signers whose
    /// failures were never scored count in `affected_signers` only.
    pub affected_signers: i64,
    pub likely_human_signers: i64,
    pub unclear_signers: i64,
    pub likely_bot_signers: i64,
}

/// One signer's failures in the window, classed by its most bot-like one.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct SignerRow {
    pub signer: String,
    pub failures: i64,
    pub programs: i64,
    pub bot_score: Option<i16>,
    pub bot_class: Option<String>,
    pub bot_reasons: Option<Vec<String>>,
    pub last_seen: Option<String>,
}

/// Failures and affected signers in a window.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct SignerSummary {
    pub failures: i64,
    pub affected_signers: i64,
    pub likely_human_signers: i64,
    pub unclear_signers: i64,
    pub likely_bot_signers: i64,
    /// Signers with no scored failure (ingested before scoring existed).
    pub unscored_signers: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignerFilter {
    pub hours: u32,
    pub program_id: Option<String>,
    /// `likely_bot`, `unclear` or `likely_human`.
    pub bot_class: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct ErrorCount {
    pub error_kind: String,
    pub error_code: Option<i64>,
    pub error_name: Option<String>,
    pub decode_source: String,
    pub failures: i64,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Bucket {
    pub start: String,
    pub failures: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct CoveragePart {
    pub decode_source: String,
    pub failures: i64,
    /// Fraction of all failures in the window, 0..=1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct FailureRow {
    pub signature: String,
    pub slot: i64,
    pub block_time: Option<String>,
    pub signer: Option<String>,
    pub fee: i64,
    pub priority_fee: i64,
    pub cu_requested: i64,
    pub cu_consumed: Option<i64>,
    pub uses_alt: bool,
    pub log_truncated: bool,
    pub top_level_ix_index: Option<i16>,
    pub root_program_id: Option<String>,
    pub failing_program_id: Option<String>,
    pub cpi_depth: Option<i32>,
    pub attribution_confidence: String,
    pub error_kind: String,
    pub error_code: Option<i64>,
    pub error_name: Option<String>,
    pub error_message: Option<String>,
    pub error_account: Option<String>,
    pub decode_source: String,
    pub cu_price: Option<i64>,
    pub jito_tip: Option<bool>,
    pub bot_score: Option<i16>,
    pub bot_class: Option<String>,
    pub bot_reasons: Option<Vec<String>>,
    pub bot_features: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureFilter {
    pub hours: u32,
    pub program_id: Option<String>,
    pub error_name: Option<String>,
    pub signer: Option<String>,
    pub bot_class: Option<String>,
}

pub trait Queries: Send + Sync {
    /// Programs with the most failures (tx-level errors have no program and
    /// are left out).
    fn top_programs(
        &self,
        hours: u32,
        page: Page,
    ) -> impl Future<Output = Result<Vec<TopProgram>>> + Send;

    /// Most frequent errors of one failing program.
    fn program_errors(
        &self,
        program_id: &str,
        hours: u32,
        page: Page,
    ) -> impl Future<Output = Result<Vec<ErrorCount>>> + Send;

    /// Failure counts per bucket, oldest first, empty buckets included.
    fn failures_over_time(
        &self,
        hours: u32,
        bucket: TimeBucket,
        program_id: Option<&str>,
    ) -> impl Future<Output = Result<Vec<Bucket>>> + Send;

    /// How many failures each decode source explained.
    fn coverage(&self, hours: u32) -> impl Future<Output = Result<Vec<CoveragePart>>> + Send;

    /// Most recent failures first.
    fn failures(
        &self,
        filter: &FailureFilter,
        page: Page,
    ) -> impl Future<Output = Result<Vec<FailureRow>>> + Send;

    fn failure(&self, signature: &str) -> impl Future<Output = Result<Option<FailureRow>>> + Send;

    /// Signers with the most failures, each classed by its most bot-like failure.
    fn top_signers(
        &self,
        filter: &SignerFilter,
        page: Page,
    ) -> impl Future<Output = Result<Vec<SignerRow>>> + Send;

    /// Failures and affected signers by class, optionally for one program.
    fn signer_summary(
        &self,
        hours: u32,
        program_id: Option<&str>,
    ) -> impl Future<Output = Result<SignerSummary>> + Send;
}

const TS: &str = "COALESCE(block_time, ingested_at)";

/// Orders classes so `max()` picks the most bot-like one.
const CLASS_RANK: &str = "CASE bot_class WHEN 'likely_human' THEN 0 WHEN 'unclear' THEN 1
                          WHEN 'likely_bot' THEN 2 END";

/// Per-signer class counts from a `signers` CTE with a `rank` column.
const CLASS_COUNTS: &str = "count(*) AS affected_signers,
         count(*) FILTER (WHERE rank = 0) AS likely_human_signers,
         count(*) FILTER (WHERE rank = 1) AS unclear_signers,
         count(*) FILTER (WHERE rank = 2) AS likely_bot_signers";

fn rfc3339(expr: &str) -> String {
    format!("to_char({expr} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')")
}

fn failure_columns() -> String {
    format!(
        "signature, slot, {} AS block_time, signer, fee, priority_fee, cu_requested,
         cu_consumed, uses_alt, log_truncated, top_level_ix_index, root_program_id,
         failing_program_id, cpi_depth, attribution_confidence, error_kind, error_code,
         error_name, error_message, error_account, decode_source, cu_price, jito_tip,
         bot_score, bot_class, bot_reasons, bot_features",
        rfc3339("block_time")
    )
}

impl Queries for PgStore {
    async fn top_programs(&self, hours: u32, page: Page) -> Result<Vec<TopProgram>> {
        let sql = format!(
            "WITH w AS (
                SELECT failing_program_id AS program_id, signer, error_kind, error_code,
                       error_name, {CLASS_RANK} AS rank, {TS} AS ts
                FROM failed_tx
                WHERE failing_program_id IS NOT NULL
                  AND {TS} >= now() - make_interval(hours => $1)
             ), signers AS (
                SELECT program_id, signer, max(rank) AS rank
                FROM w WHERE signer IS NOT NULL GROUP BY 1, 2
             ), classes AS (
                SELECT program_id, {CLASS_COUNTS} FROM signers GROUP BY 1
             ), totals AS (
                SELECT program_id, count(*) AS failures,
                       count(DISTINCT (error_kind, error_code, error_name)) AS distinct_errors,
                       max(ts) AS last_ts
                FROM w GROUP BY 1
             )
             SELECT t.program_id, t.failures, t.distinct_errors, {last_seen} AS last_seen,
                    COALESCE(c.affected_signers, 0) AS affected_signers,
                    COALESCE(c.likely_human_signers, 0) AS likely_human_signers,
                    COALESCE(c.unclear_signers, 0) AS unclear_signers,
                    COALESCE(c.likely_bot_signers, 0) AS likely_bot_signers
             FROM totals t LEFT JOIN classes c USING (program_id)
             ORDER BY t.failures DESC, t.program_id
             LIMIT $2 OFFSET $3",
            last_seen = rfc3339("t.last_ts"),
        );
        Ok(sqlx::query_as(&sql)
            .bind(hours as i32)
            .bind(i64::from(page.limit))
            .bind(i64::from(page.offset))
            .fetch_all(self.pool())
            .await?)
    }

    async fn program_errors(
        &self,
        program_id: &str,
        hours: u32,
        page: Page,
    ) -> Result<Vec<ErrorCount>> {
        let sql = format!(
            "SELECT error_kind, error_code, error_name, decode_source,
                    count(*) AS failures, {last_seen} AS last_seen
             FROM failed_tx
             WHERE failing_program_id = $1 AND {TS} >= now() - make_interval(hours => $2)
             GROUP BY error_kind, error_code, error_name, decode_source
             ORDER BY failures DESC, error_code NULLS LAST, error_name
             LIMIT $3 OFFSET $4",
            last_seen = rfc3339(&format!("max({TS})")),
        );
        Ok(sqlx::query_as(&sql)
            .bind(program_id)
            .bind(hours as i32)
            .bind(i64::from(page.limit))
            .bind(i64::from(page.offset))
            .fetch_all(self.pool())
            .await?)
    }

    async fn failures_over_time(
        &self,
        hours: u32,
        bucket: TimeBucket,
        program_id: Option<&str>,
    ) -> Result<Vec<Bucket>> {
        let unit = bucket.sql();
        let sql = format!(
            "WITH buckets AS (
                SELECT generate_series(
                    date_trunc('{unit}', now() - make_interval(hours => $1), 'UTC'),
                    date_trunc('{unit}', now(), 'UTC'),
                    interval '1 {unit}') AS b
             ), counts AS (
                SELECT date_trunc('{unit}', {TS}, 'UTC') AS b, count(*) AS n
                FROM failed_tx
                WHERE {TS} >= now() - make_interval(hours => $1)
                  AND ($2::text IS NULL OR failing_program_id = $2)
                GROUP BY 1
             )
             SELECT {start} AS start, COALESCE(counts.n, 0) AS failures
             FROM buckets LEFT JOIN counts USING (b)
             ORDER BY buckets.b",
            start = rfc3339("buckets.b"),
        );
        Ok(sqlx::query_as(&sql)
            .bind(hours as i32)
            .bind(program_id)
            .fetch_all(self.pool())
            .await?)
    }

    async fn coverage(&self, hours: u32) -> Result<Vec<CoveragePart>> {
        let sql = format!(
            "SELECT decode_source, count(*) AS failures,
                    (count(*)::float8 / sum(count(*)) OVER ()) AS share
             FROM failed_tx
             WHERE {TS} >= now() - make_interval(hours => $1)
             GROUP BY decode_source
             ORDER BY failures DESC, decode_source"
        );
        Ok(sqlx::query_as(&sql)
            .bind(hours as i32)
            .fetch_all(self.pool())
            .await?)
    }

    async fn failures(&self, filter: &FailureFilter, page: Page) -> Result<Vec<FailureRow>> {
        let sql = format!(
            "SELECT {cols} FROM failed_tx
             WHERE {TS} >= now() - make_interval(hours => $1)
               AND ($2::text IS NULL OR failing_program_id = $2)
               AND ($3::text IS NULL OR error_name = $3)
               AND ($4::text IS NULL OR signer = $4)
               AND ($5::text IS NULL OR bot_class = $5)
             ORDER BY slot DESC, signature
             LIMIT $6 OFFSET $7",
            cols = failure_columns(),
        );
        Ok(sqlx::query_as(&sql)
            .bind(filter.hours as i32)
            .bind(&filter.program_id)
            .bind(&filter.error_name)
            .bind(&filter.signer)
            .bind(&filter.bot_class)
            .bind(i64::from(page.limit))
            .bind(i64::from(page.offset))
            .fetch_all(self.pool())
            .await?)
    }

    async fn failure(&self, signature: &str) -> Result<Option<FailureRow>> {
        let sql = format!(
            "SELECT {} FROM failed_tx WHERE signature = $1",
            failure_columns()
        );
        Ok(sqlx::query_as(&sql)
            .bind(signature)
            .fetch_optional(self.pool())
            .await?)
    }

    async fn top_signers(&self, filter: &SignerFilter, page: Page) -> Result<Vec<SignerRow>> {
        let sql = format!(
            "WITH w AS (
                SELECT signer, failing_program_id, bot_score, bot_class, bot_reasons,
                       {CLASS_RANK} AS rank, {TS} AS ts
                FROM failed_tx
                WHERE signer IS NOT NULL
                  AND {TS} >= now() - make_interval(hours => $1)
                  AND ($2::text IS NULL OR failing_program_id = $2)
             ), agg AS (
                SELECT signer, count(*) AS failures,
                       count(DISTINCT failing_program_id) AS programs, max(ts) AS last_ts
                FROM w GROUP BY signer
             ), worst AS (
                SELECT DISTINCT ON (signer) signer, bot_score, bot_class, bot_reasons
                FROM w ORDER BY signer, rank DESC NULLS LAST, bot_score DESC NULLS LAST
             )
             SELECT a.signer, a.failures, a.programs, x.bot_score, x.bot_class,
                    x.bot_reasons, {last_seen} AS last_seen
             FROM agg a JOIN worst x USING (signer)
             WHERE ($3::text IS NULL OR x.bot_class = $3)
             ORDER BY a.failures DESC, a.signer
             LIMIT $4 OFFSET $5",
            last_seen = rfc3339("a.last_ts"),
        );
        Ok(sqlx::query_as(&sql)
            .bind(filter.hours as i32)
            .bind(&filter.program_id)
            .bind(&filter.bot_class)
            .bind(i64::from(page.limit))
            .bind(i64::from(page.offset))
            .fetch_all(self.pool())
            .await?)
    }

    async fn signer_summary(&self, hours: u32, program_id: Option<&str>) -> Result<SignerSummary> {
        let sql = format!(
            "WITH w AS (
                SELECT signer, {CLASS_RANK} AS rank
                FROM failed_tx
                WHERE {TS} >= now() - make_interval(hours => $1)
                  AND ($2::text IS NULL OR failing_program_id = $2)
             ), signers AS (
                SELECT signer, max(rank) AS rank FROM w WHERE signer IS NOT NULL GROUP BY 1
             )
             SELECT (SELECT count(*) FROM w) AS failures, {CLASS_COUNTS},
                    count(*) FILTER (WHERE rank IS NULL) AS unscored_signers
             FROM signers"
        );
        Ok(sqlx::query_as(&sql)
            .bind(hours as i32)
            .bind(program_id)
            .fetch_one(self.pool())
            .await?)
    }
}
