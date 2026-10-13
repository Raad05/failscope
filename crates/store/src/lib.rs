//! Storage behind a trait, with a Postgres implementation.
//!
//! Queries are checked at runtime (no `sqlx::query!` macros), so building
//! never needs a database. `tests/postgres.rs` runs them against a real one.

use std::future::Future;

use failscope_decoder::DecodedFailure;
use failscope_score::{BotScore, Observation};
use sqlx::postgres::{PgPool, PgPoolOptions};

mod alerts;
mod queries;

pub use alerts::{AlertEvent, AlertMetric, AlertState, AlertStore, NewAlertEvent};

pub use queries::{
    Bucket, CoveragePart, ErrorCount, FailureFilter, FailureRow, Page, Queries, SignerFilter,
    SignerRow, SignerSummary, TimeBucket, TopProgram, MAX_LIMIT,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0} out of range for storage: {1}")]
    Range(&'static str, u64),
    #[error("could not serialize {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub trait Store: Send + Sync {
    /// Inserts a decoded failure with its signer's bot score, if any.
    /// Returns false if the signature was already stored.
    fn insert_failure(
        &self,
        failure: &DecodedFailure,
        bot: Option<&BotScore>,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Stored failures from `min_slot` on, oldest first, as the scoring
    /// tracker sees them. Used to rebuild its windows after a restart.
    fn recent_observations(
        &self,
        min_slot: u64,
    ) -> impl Future<Output = Result<Vec<Observation>>> + Send;

    /// Fills `block_time` for rows of `slot` that don't have one yet.
    fn set_block_time(&self, slot: u64, unix_time: i64)
        -> impl Future<Output = Result<u64>> + Send;

    fn cursor(&self, stream: &str) -> impl Future<Output = Result<Option<u64>>> + Send;

    fn save_cursor(&self, stream: &str, slot: u64) -> impl Future<Output = Result<()>> + Send;

    fn record_gap(
        &self,
        stream: &str,
        from_slot: u64,
        to_slot: u64,
        reason: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Inserts or replaces a program's cached IDL lookup.
    fn upsert_idl(&self, row: &IdlCacheRow) -> impl Future<Output = Result<()>> + Send;

    fn load_idls(&self) -> impl Future<Output = Result<Vec<IdlCacheRow>>> + Send;
}

/// One persisted IDL lookup. Plain strings, so the store doesn't depend on
/// the IDL crate; ingest converts.
#[derive(Debug, Clone, PartialEq)]
pub struct IdlCacheRow {
    pub program_id: String,
    /// `found` or `missing`.
    pub fetch_status: String,
    pub location: Option<String>,
    pub idl_format: Option<String>,
    pub idl_json: Option<serde_json::Value>,
    /// Seconds since the fetch.
    pub age_secs: u64,
}

#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

fn to_i64(what: &'static str, v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::Range(what, v))
}

fn enum_text<T: serde::Serialize>(what: &'static str, v: &T) -> Result<String> {
    match serde_json::to_value(v).map_err(|e| StoreError::Serialize(what, e))? {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

impl PgStore {
    /// Connects and runs pending migrations.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new().max_connections(5).connect(url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl Store for PgStore {
    async fn insert_failure(&self, f: &DecodedFailure, bot: Option<&BotScore>) -> Result<bool> {
        let features = bot
            .map(|b| serde_json::to_value(&b.features))
            .transpose()
            .map_err(|e| StoreError::Serialize("bot_features", e))?;
        let reasons: Option<Vec<&str>> =
            bot.map(|b| b.reasons.iter().map(|r| r.as_str()).collect());
        let result = sqlx::query(
            "INSERT INTO failed_tx (
                signature, slot, block_time, fee, priority_fee, cu_requested, cu_consumed,
                signer, uses_alt, log_truncated, top_level_ix_index, root_program_id,
                failing_program_id, cpi_depth, attribution_confidence, error_kind, error_code,
                error_name, error_message, error_account, decode_source, raw_err,
                cu_price, cu_limit_explicit, jito_tip,
                bot_score, bot_class, bot_reasons, bot_features)
             VALUES ($1, $2, to_timestamp($3), $4, $5, $6, $7, $8, $9, $10, $11, $12,
                     $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
                     $23, $24, $25, $26, $27, $28, $29)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(&f.signature)
        .bind(to_i64("slot", f.slot)?)
        .bind(f.block_time.map(|t| t as f64))
        .bind(to_i64("fee", f.fee)?)
        .bind(to_i64("priority_fee", f.priority_fee)?)
        .bind(i64::from(f.cu_requested))
        .bind(
            f.cu_consumed
                .map(|c| to_i64("cu_consumed", c))
                .transpose()?,
        )
        .bind(&f.signer)
        .bind(f.uses_alt)
        .bind(f.log_truncated)
        .bind(f.top_level_ix_index.map(i16::from))
        .bind(&f.root_program_id)
        .bind(&f.failing_program_id)
        .bind(f.cpi_depth.map(|d| i32::try_from(d).unwrap_or(i32::MAX)))
        .bind(enum_text(
            "attribution_confidence",
            &f.attribution_confidence,
        )?)
        .bind(&f.error_kind)
        .bind(f.error_code.map(i64::from))
        .bind(&f.error_name)
        .bind(&f.error_message)
        .bind(&f.error_account)
        .bind(enum_text("decode_source", &f.decode_source)?)
        .bind(&f.raw_err)
        // Prices are capped rather than rejected: only percentiles use them.
        .bind(i64::try_from(f.cu_price).unwrap_or(i64::MAX))
        .bind(f.cu_limit_explicit)
        .bind(f.jito_tip)
        .bind(bot.map(|b| i16::from(b.score)))
        .bind(bot.map(|b| b.class.as_str()))
        .bind(reasons)
        .bind(features)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn recent_observations(&self, min_slot: u64) -> Result<Vec<Observation>> {
        type Row = (
            String,
            String,
            i64,
            Option<String>,
            String,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<bool>,
            String,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT signature, signer, slot, failing_program_id, error_kind, error_code,
                    error_name, cu_price, jito_tip, decode_source
             FROM failed_tx
             WHERE slot >= $1 AND signer IS NOT NULL
             ORDER BY slot, signature",
        )
        .bind(to_i64("slot", min_slot)?)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(signature, signer, slot, program, kind, code, name, price, tip, source)| {
                    Observation {
                        signature,
                        signer,
                        slot: u64::try_from(slot).unwrap_or(0),
                        program,
                        error_key: failscope_score::error_key(&kind, code, name.as_deref()),
                        cu_price: price.and_then(|p| u64::try_from(p).ok()).unwrap_or(0),
                        jito_tip: tip.unwrap_or(false),
                        silent_program: source == "unknown",
                    }
                },
            )
            .collect())
    }

    async fn set_block_time(&self, slot: u64, unix_time: i64) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE failed_tx SET block_time = to_timestamp($2)
             WHERE slot = $1 AND block_time IS NULL",
        )
        .bind(to_i64("slot", slot)?)
        .bind(unix_time as f64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    async fn cursor(&self, stream: &str) -> Result<Option<u64>> {
        let slot: Option<i64> =
            sqlx::query_scalar("SELECT slot FROM ingest_cursor WHERE stream = $1")
                .bind(stream)
                .fetch_optional(&self.pool)
                .await?;
        Ok(slot.and_then(|s| u64::try_from(s).ok()))
    }

    async fn save_cursor(&self, stream: &str, slot: u64) -> Result<()> {
        // Never moves backwards, even if an older save lands late.
        sqlx::query(
            "INSERT INTO ingest_cursor (stream, slot) VALUES ($1, $2)
             ON CONFLICT (stream) DO UPDATE
             SET slot = GREATEST(ingest_cursor.slot, EXCLUDED.slot), updated_at = now()",
        )
        .bind(stream)
        .bind(to_i64("slot", slot)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn record_gap(
        &self,
        stream: &str,
        from_slot: u64,
        to_slot: u64,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO ingest_gaps (stream, from_slot, to_slot, reason) VALUES ($1, $2, $3, $4)",
        )
        .bind(stream)
        .bind(to_i64("from_slot", from_slot)?)
        .bind(to_i64("to_slot", to_slot)?)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_idl(&self, row: &IdlCacheRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO idl_cache (program_id, fetch_status, location, idl_format, idl_json, fetched_at)
             VALUES ($1, $2, $3, $4, $5, now() - make_interval(secs => $6))
             ON CONFLICT (program_id) DO UPDATE SET
                fetch_status = EXCLUDED.fetch_status, location = EXCLUDED.location,
                idl_format = EXCLUDED.idl_format, idl_json = EXCLUDED.idl_json,
                fetched_at = EXCLUDED.fetched_at",
        )
        .bind(&row.program_id)
        .bind(&row.fetch_status)
        .bind(&row.location)
        .bind(&row.idl_format)
        .bind(&row.idl_json)
        .bind(row.age_secs as f64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_idls(&self) -> Result<Vec<IdlCacheRow>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            program_id: String,
            fetch_status: String,
            location: Option<String>,
            idl_format: Option<String>,
            idl_json: Option<serde_json::Value>,
            age: f64,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT program_id, fetch_status, location, idl_format, idl_json,
                    GREATEST(extract(epoch FROM now() - fetched_at), 0)::float8 AS age
             FROM idl_cache",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| IdlCacheRow {
                program_id: r.program_id,
                fetch_status: r.fetch_status,
                location: r.location,
                idl_format: r.idl_format,
                idl_json: r.idl_json,
                age_secs: r.age as u64,
            })
            .collect())
    }
}
