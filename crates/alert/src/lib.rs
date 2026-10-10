//! Spike alerts on per-program failure rates, delivered by webhook.
//!
//! Every tick: count each program's failures per minute, evaluate the rule
//! (see `rule`), and on a state change (firing / resolved) POST a webhook.
//! The change is recorded only after the webhook accepted it, so a failed
//! delivery is retried on the next tick instead of being lost.

mod rule;

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use failscope_store::{AlertState, AlertStore, NewAlertEvent, Page, Queries};
use serde::Serialize;

pub use rule::{Action, Evaluation, Rule};

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub rule: Rule,
    /// Secret: never logged.
    pub webhook_url: String,
    pub interval: Duration,
    /// Programs to watch; empty = every program that fails.
    pub programs: Vec<String>,
    /// Base URL of the dashboard, for a link in the alert.
    pub dashboard_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AlertError {
    #[error("store: {0}")]
    Store(#[from] failscope_store::StoreError),
    #[error("webhook delivery failed after {attempts} attempts: {last}")]
    Webhook { attempts: u32, last: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct TopError {
    pub error_name: Option<String>,
    pub error_code: Option<i64>,
    pub failures: i64,
}

/// Webhook body. `text` makes it readable as-is in Slack-style incoming
/// webhooks; the other fields are for machines.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    pub source: &'static str,
    pub state: AlertState,
    pub program_id: String,
    pub window_minutes: u32,
    pub current: u64,
    pub baseline_per_window: f64,
    pub threshold: f64,
    pub top_errors: Vec<TopError>,
    pub dashboard_url: Option<String>,
    pub at_unix: u64,
    pub text: String,
}

/// Runs ticks every `config.interval` until `shutdown` resolves.
pub async fn run<S>(
    config: AlertConfig,
    store: Arc<S>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), AlertError>
where
    S: AlertStore + Queries + 'static,
{
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AlertError::Webhook {
            attempts: 0,
            last: e.to_string(),
        })?;
    tracing::info!(
        window = config.rule.window_minutes,
        baseline = config.rule.baseline_minutes,
        factor = config.rule.factor,
        min_failures = config.rule.min_failures,
        programs = config.programs.len(),
        "alerting started"
    );
    tokio::pin!(shutdown);
    loop {
        match tick(&config, store.as_ref(), &client).await {
            Ok(events) => {
                for e in events {
                    tracing::info!(program = %e.program_id, state = ?e.state, current = e.current_count, "alert delivered");
                }
            }
            Err(e) => tracing::warn!(error = %e, "alert tick failed; will retry"),
        }
        tokio::select! {
            () = tokio::time::sleep(config.interval) => {}
            () = &mut shutdown => {
                tracing::info!("alerting stopped");
                return Ok(());
            }
        }
    }
}

/// One evaluation pass. Returns the state changes that were delivered and recorded.
pub async fn tick<S: AlertStore + Queries>(
    config: &AlertConfig,
    store: &S,
    client: &reqwest::Client,
) -> Result<Vec<NewAlertEvent>, AlertError> {
    let rule = &config.rule;
    let counts = store
        .failures_per_minute(rule.span_minutes(), &config.programs)
        .await?;
    let states = store.alert_states().await?;

    // Programs with recent failures, plus firing ones (which may now be at zero).
    let watched = |p: &String| config.programs.is_empty() || config.programs.contains(p);
    let candidates: BTreeSet<&String> = counts
        .keys()
        .chain(
            states
                .iter()
                .filter(|(_, s)| **s == AlertState::Firing)
                .map(|(p, _)| p),
        )
        .filter(|p| watched(p))
        .collect();

    let mut delivered = Vec::new();
    let mut first_error = None;
    for program in candidates {
        let evaluation = rule.evaluate(counts.get(program).map_or(&[][..], Vec::as_slice));
        let state = match rule.decide(states.get(program).copied(), &evaluation) {
            Action::Fire => AlertState::Firing,
            Action::Resolve => AlertState::Resolved,
            Action::None => continue,
        };
        let event = NewAlertEvent {
            program_id: program.clone(),
            state,
            window_minutes: rule.window_minutes,
            current_count: evaluation.current,
            baseline_per_window: evaluation.baseline_per_window,
            threshold: evaluation.threshold,
        };
        let payload = payload(config, store, &event).await?;
        match deliver(client, &config.webhook_url, &payload).await {
            Ok(()) => {
                store.record_alert(&event).await?;
                delivered.push(event);
            }
            // Not recorded: the same transition is retried next tick.
            Err(e) => {
                tracing::warn!(program = %program, error = %e, "webhook failed");
                first_error.get_or_insert(e);
            }
        }
    }
    match first_error {
        Some(e) if delivered.is_empty() => Err(e),
        _ => Ok(delivered),
    }
}

async fn payload<S: Queries>(
    config: &AlertConfig,
    store: &S,
    e: &NewAlertEvent,
) -> Result<WebhookPayload, AlertError> {
    let top_errors = store
        .program_errors(
            &e.program_id,
            1,
            Page {
                limit: 3,
                offset: 0,
            },
        )
        .await?
        .into_iter()
        .map(|r| TopError {
            error_name: r.error_name,
            error_code: r.error_code,
            failures: r.failures,
        })
        .collect::<Vec<_>>();
    let dashboard_url = config.dashboard_url.as_ref().map(|base| {
        format!(
            "{}/#hours=1&program={}",
            base.trim_end_matches('/'),
            e.program_id
        )
    });
    let top = top_errors
        .iter()
        .map(|t| {
            format!(
                "{} ({})",
                t.error_name.as_deref().unwrap_or("unknown"),
                t.failures
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let text = match e.state {
        AlertState::Firing => format!(
            "failscope: failures spiking for {}: {} in the last {} min (baseline {:.1} per {} min, threshold {:.0}). Top errors: {}",
            e.program_id, e.current_count, e.window_minutes, e.baseline_per_window, e.window_minutes, e.threshold,
            if top.is_empty() { "none decoded".to_string() } else { top },
        ),
        AlertState::Resolved => format!(
            "failscope: resolved for {}: {} failures in the last {} min (threshold {:.0})",
            e.program_id, e.current_count, e.window_minutes, e.threshold,
        ),
    };
    Ok(WebhookPayload {
        source: "failscope",
        state: e.state,
        program_id: e.program_id.clone(),
        window_minutes: e.window_minutes,
        current: e.current_count,
        baseline_per_window: e.baseline_per_window,
        threshold: e.threshold,
        top_errors,
        dashboard_url,
        at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        text,
    })
}

/// POSTs the payload; 3 attempts, 1s then 2s apart. Any 2xx is success.
async fn deliver(
    client: &reqwest::Client,
    url: &str,
    payload: &WebhookPayload,
) -> Result<(), AlertError> {
    const ATTEMPTS: u32 = 3;
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match client.post(url).json(payload).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => last = format!("HTTP {}", r.status()),
            // Strip the URL from the error: it may carry a secret token.
            Err(e) => last = e.without_url().to_string(),
        }
        if attempt < ATTEMPTS {
            tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
        }
    }
    Err(AlertError::Webhook {
        attempts: ATTEMPTS,
        last,
    })
}
