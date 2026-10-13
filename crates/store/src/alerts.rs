//! Storage for the alerting loop: recent per-minute counts (failures, or
//! likely-human signers), and the log of alert state changes.

use std::collections::HashMap;
use std::future::Future;

use serde::Serialize;

use crate::{PgStore, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Firing,
    Resolved,
}

impl AlertState {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertState::Firing => "firing",
            AlertState::Resolved => "resolved",
        }
    }
}

/// What an alert counts per minute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertMetric {
    /// Failed transactions.
    #[default]
    Failures,
    /// Distinct likely-human signers with a failure, per minute. Summed over
    /// a window this counts signer-minutes: a person failing in two minutes
    /// counts twice. Bots barely move it however often they fail.
    HumanSigners,
}

impl AlertMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertMetric::Failures => "failures",
            AlertMetric::HumanSigners => "human_signers",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewAlertEvent {
    pub program_id: String,
    pub metric: AlertMetric,
    pub state: AlertState,
    pub window_minutes: u32,
    pub current_count: u64,
    pub baseline_per_window: f64,
    pub threshold: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct AlertEvent {
    pub id: i64,
    pub program_id: String,
    pub metric: String,
    pub state: String,
    pub at: String,
    pub window_minutes: i32,
    pub current_count: i64,
    pub baseline_per_window: f64,
    pub threshold: f64,
}

pub trait AlertStore: Send + Sync {
    /// `metric` per program per minute for the last `minutes` minutes.
    /// Index 0 is the current (partial) minute, index 1 the one before, ...
    fn per_minute(
        &self,
        metric: AlertMetric,
        minutes: u32,
        programs: &[String],
    ) -> impl Future<Output = Result<HashMap<String, Vec<u64>>>> + Send;

    /// Latest state per program that has ever alerted on `metric`.
    fn alert_states(
        &self,
        metric: AlertMetric,
    ) -> impl Future<Output = Result<HashMap<String, AlertState>>> + Send;

    fn record_alert(&self, event: &NewAlertEvent) -> impl Future<Output = Result<()>> + Send;

    /// Alert events in the last `hours`, newest first.
    fn alert_events(
        &self,
        hours: u32,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<AlertEvent>>> + Send;
}

impl AlertStore for PgStore {
    async fn per_minute(
        &self,
        metric: AlertMetric,
        minutes: u32,
        programs: &[String],
    ) -> Result<HashMap<String, Vec<u64>>> {
        let (count, only) = match metric {
            AlertMetric::Failures => ("count(*)", "TRUE"),
            AlertMetric::HumanSigners => (
                "count(DISTINCT signer)",
                "bot_class = 'likely_human' AND signer IS NOT NULL",
            ),
        };
        // Minutes are counted back from now (age 0 = the current minute), so
        // windows slide with the clock instead of snapping to minute marks.
        let sql = format!(
            "SELECT failing_program_id,
                    floor(extract(epoch FROM now() - COALESCE(block_time, ingested_at)) / 60)::int AS age,
                    {count}
             FROM failed_tx
             WHERE failing_program_id IS NOT NULL AND {only}
               AND COALESCE(block_time, ingested_at) > now() - make_interval(mins => $1)
               AND (cardinality($2::text[]) = 0 OR failing_program_id = ANY($2))
             GROUP BY 1, 2"
        );
        let rows: Vec<(String, i32, i64)> = sqlx::query_as(&sql)
            .bind(minutes as i32)
            .bind(programs)
            .fetch_all(self.pool())
            .await?;

        let len = minutes as usize;
        let mut out: HashMap<String, Vec<u64>> = HashMap::new();
        for (program, age, count) in rows {
            let Ok(age) = usize::try_from(age) else {
                continue;
            };
            if age >= len {
                continue;
            }
            let counts = out.entry(program).or_insert_with(|| vec![0; len]);
            counts[age] += u64::try_from(count).unwrap_or(0);
        }
        Ok(out)
    }

    async fn alert_states(&self, metric: AlertMetric) -> Result<HashMap<String, AlertState>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT ON (program_id) program_id, state
             FROM alert_events WHERE metric = $1
             ORDER BY program_id, at DESC, id DESC",
        )
        .bind(metric.as_str())
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(p, s)| {
                let state = if s == "firing" {
                    AlertState::Firing
                } else {
                    AlertState::Resolved
                };
                (p, state)
            })
            .collect())
    }

    async fn record_alert(&self, e: &NewAlertEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO alert_events
                (program_id, state, window_minutes, current_count, baseline_per_window,
                 threshold, metric)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&e.program_id)
        .bind(e.state.as_str())
        .bind(e.window_minutes as i32)
        .bind(crate::to_i64("current_count", e.current_count)?)
        .bind(e.baseline_per_window)
        .bind(e.threshold)
        .bind(e.metric.as_str())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn alert_events(&self, hours: u32, limit: u32) -> Result<Vec<AlertEvent>> {
        Ok(sqlx::query_as(
            "SELECT id, program_id, metric, state,
                    to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS at,
                    window_minutes, current_count, baseline_per_window, threshold
             FROM alert_events
             WHERE at >= now() - make_interval(hours => $1)
             ORDER BY at DESC, id DESC
             LIMIT $2",
        )
        .bind(hours as i32)
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await?)
    }
}
