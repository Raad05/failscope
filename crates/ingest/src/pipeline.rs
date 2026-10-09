//! Producer: Yellowstone stream -> bounded channel. Consumer: decode + store.
//!
//! The channel is bounded, so a slow database slows the producer, which
//! stops reading the stream. If the server then drops us, we reconnect and
//! resume from the cursor.
//!
//! Cursor = last slot whose block meta was processed. Messages for a slot
//! arrive before its block meta and the consumer handles events in order,
//! so every failure up to the cursor is stored. On reconnect we ask for
//! `from_slot = cursor`. That slot is replayed again, and inserts are
//! idempotent (signature primary key).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use failscope_decoder::{decode, DecodeSource, DecodedFailure, TxInput};
use failscope_idl::{
    detect_format, AccountSource, FetchStatus, Idl, IdlCache, IdlFormat, IdlLocation,
};
use failscope_store::{IdlCacheRow, Store};
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, Instant};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterTransactions, SubscribeRequestPing,
    SubscribeUpdateTransactionInfo,
};
use yellowstone_grpc_proto::tonic::Status;

use crate::convert::tx_input;

#[derive(Debug, Clone)]
pub struct IngestConfig {
    pub endpoint: String,
    pub x_token: Option<String>,
    /// Name of this stream's cursor row.
    pub stream: String,
    pub channel_capacity: usize,
    pub cursor_flush_interval: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
}

impl IngestConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            x_token: None,
            stream: "default".to_string(),
            channel_capacity: 1024,
            cursor_flush_interval: Duration::from_secs(2),
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("store: {0}")]
    Store(#[from] failscope_store::StoreError),
    #[error("consumer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Debug)]
enum Event {
    Tx {
        slot: u64,
        info: Box<SubscribeUpdateTransactionInfo>,
    },
    BlockMeta {
        slot: u64,
        block_time: Option<i64>,
    },
}

/// Counters, logged periodically and returned at shutdown.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IngestStats {
    pub inserted: u64,
    pub duplicates: u64,
    pub convert_errors: u64,
    pub reconnects: u64,
}

/// Runs until `shutdown` resolves, then drains buffered events, saves the
/// cursor, and returns the counters.
pub async fn run<St, S>(
    config: IngestConfig,
    store: Arc<St>,
    idls: Arc<IdlCache<S>>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<IngestStats, IngestError>
where
    St: Store + 'static,
    S: AccountSource + 'static,
{
    let (tx, rx) = mpsc::channel(config.channel_capacity);
    let (stop_tx, stop_rx) = watch::channel(false);

    let seeded = seed_idls(store.as_ref(), &idls).await?;
    tracing::info!(seeded, "IDL cache loaded from store");
    let consumer = tokio::spawn(consume(config.clone(), Arc::clone(&store), idls, rx));
    let producer = produce(config, store, tx, stop_rx);

    tokio::pin!(producer);
    let reconnects = tokio::select! {
        r = &mut producer => r,
        () = shutdown => {
            tracing::info!("shutdown requested");
            let _ = stop_tx.send(true);
            producer.await
        }
    };
    // Producer dropped its sender: the consumer drains what's buffered and exits.
    let mut stats = consumer.await??;
    stats.reconnects = reconnects;
    Ok(stats)
}

// ---------------------------------------------------------------- producer

enum StreamEnd {
    Stopped,
    Closed,
}

/// What earlier attempts learned about the server's replay window.
#[derive(Debug, Default)]
struct ReplayState {
    /// The server refused our `from_slot`; this is its oldest stored slot.
    oldest: Option<u64>,
    /// The server doesn't support `from_slot` at all.
    unsupported: bool,
}

async fn produce<St: Store>(
    config: IngestConfig,
    store: Arc<St>,
    events: mpsc::Sender<Event>,
    mut stop: watch::Receiver<bool>,
) -> u64 {
    let mut backoff = config.backoff_min;
    let mut connections = 0u64;
    let mut replay = ReplayState::default();
    loop {
        if *stop.borrow() {
            break;
        }
        connections += 1;
        let started = Instant::now();
        match stream_once(&config, store.as_ref(), &events, &mut stop, &mut replay).await {
            Ok(StreamEnd::Stopped) => break,
            Ok(StreamEnd::Closed) => tracing::warn!("stream closed by server"),
            Err(StreamError::Status(status)) if replay_refused(&status, &mut replay) => {
                // Retry at once with the adjusted start; nothing is wrong with the server.
                tracing::warn!(message = status.message(), "from_slot refused, adjusting");
                continue;
            }
            Err(e) => tracing::warn!(error = %e, "stream failed"),
        }
        // A connection that lasted a while resets the backoff.
        if started.elapsed() > config.backoff_max {
            backoff = config.backoff_min;
        }
        tracing::info!(?backoff, "reconnecting");
        tokio::select! {
            () = sleep(backoff) => {}
            _ = stop.changed() => break,
        }
        backoff = (backoff * 2).min(config.backoff_max);
    }
    connections.saturating_sub(1)
}

/// Recognizes the server refusing `from_slot` (yellowstone-grpc 12.x
/// messages) and records what to do next. Returns false for other errors.
fn replay_refused(status: &Status, replay: &mut ReplayState) -> bool {
    let message = status.message();
    if message.contains("from_slot is not supported") {
        replay.unsupported = true;
        return true;
    }
    // "broadcast from {from} is not available, last available: {oldest}"
    if let Some(oldest) = message
        .split_once("is not available, last available: ")
        .and_then(|(_, n)| n.trim().parse().ok())
    {
        replay.oldest = Some(oldest);
        return true;
    }
    false
}

#[derive(Debug, thiserror::Error)]
enum StreamError {
    #[error("connect: {0}")]
    Connect(String),
    #[error("subscribe: {0}")]
    Subscribe(String),
    #[error("stream: {0}")]
    Status(#[from] Status),
    #[error("store: {0}")]
    Store(#[from] failscope_store::StoreError),
}

async fn stream_once<St: Store>(
    config: &IngestConfig,
    store: &St,
    events: &mpsc::Sender<Event>,
    stop: &mut watch::Receiver<bool>,
    replay: &mut ReplayState,
) -> Result<StreamEnd, StreamError> {
    let mut client = GeyserGrpcClient::build_from_shared(config.endpoint.clone())
        .and_then(|b| b.x_token(config.x_token.clone()))
        .map_err(|e| StreamError::Connect(e.to_string()))?
        .connect_timeout(Duration::from_secs(10))
        .max_decoding_message_size(64 * 1024 * 1024)
        .connect()
        .await
        .map_err(|e| StreamError::Connect(e.to_string()))?;

    // Where to resume. The cursor slot itself is replayed (inserts are
    // idempotent). If the server can't go back that far, record the gap.
    let cursor = store.cursor(&config.stream).await?;
    let first_available = if replay.unsupported {
        None
    } else {
        match client.subscribe_replay_info().await {
            // u64::MAX = nothing pruned yet: everything since server start is
            // stored, so the cursor is worth trying; the server refuses it if not.
            Ok(info) => info.first_available.filter(|&s| s != u64::MAX),
            Err(e) => {
                tracing::warn!(error = %e, "replay info unavailable");
                None
            }
        }
    };
    let oldest = replay.oldest.take().or(first_available);
    let (from_slot, mut gap_check) = match cursor {
        None => (None, None),
        // Can't replay: compare the cursor with the first slot we receive.
        Some(c) if replay.unsupported => (None, Some(c)),
        Some(c) => match oldest {
            Some(oldest) if oldest > c => {
                if c + 1 < oldest {
                    store
                        .record_gap(
                            &config.stream,
                            c + 1,
                            oldest - 1,
                            "older than server replay window",
                        )
                        .await?;
                    tracing::warn!(
                        from = c + 1,
                        to = oldest - 1,
                        "gap: slots no longer replayable"
                    );
                }
                (Some(oldest), None)
            }
            _ => (Some(c), None),
        },
    };
    tracing::info!(?cursor, ?first_available, ?from_slot, "subscribing");

    let request = SubscribeRequest {
        transactions: HashMap::from([(
            "failed".to_string(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(true),
                ..Default::default()
            },
        )]),
        blocks_meta: HashMap::from([("meta".to_string(), SubscribeRequestFilterBlocksMeta {})]),
        commitment: Some(CommitmentLevel::Confirmed as i32),
        from_slot,
        ..Default::default()
    };
    let (mut sink, mut stream) = client
        .subscribe_with_request(Some(request))
        .await
        .map_err(|e| StreamError::Subscribe(e.to_string()))?;

    loop {
        let message = tokio::select! {
            m = stream.next() => m,
            _ = stop.changed() => return Ok(StreamEnd::Stopped),
        };
        let Some(update) = message.transpose()? else {
            return Ok(StreamEnd::Closed);
        };
        let event = match update.update_oneof {
            Some(UpdateOneof::Transaction(t)) => match t.transaction {
                Some(info) => Event::Tx {
                    slot: t.slot,
                    info: Box::new(info),
                },
                None => continue,
            },
            Some(UpdateOneof::BlockMeta(m)) => Event::BlockMeta {
                slot: m.slot,
                block_time: m.block_time.map(|t| t.timestamp),
            },
            Some(UpdateOneof::Ping(_)) => {
                // Answer server pings so load balancers keep the stream open.
                let ping = SubscribeRequest {
                    ping: Some(SubscribeRequestPing { id: 1 }),
                    ..Default::default()
                };
                if let Err(e) = sink.send(ping).await {
                    tracing::warn!(error = %e, "could not answer ping");
                }
                continue;
            }
            _ => continue,
        };

        if let Some(cursor) = gap_check.take() {
            let first_seen = match &event {
                Event::Tx { slot, .. } | Event::BlockMeta { slot, .. } => *slot,
            };
            if first_seen > cursor + 1 {
                store
                    .record_gap(
                        &config.stream,
                        cursor + 1,
                        first_seen - 1,
                        "server has no replay",
                    )
                    .await?;
                tracing::warn!(
                    from = cursor + 1,
                    to = first_seen - 1,
                    "gap: no replay support"
                );
            }
        }

        // Backpressure: waits while the consumer is behind.
        tokio::select! {
            r = events.send(event) => if r.is_err() { return Ok(StreamEnd::Stopped) },
            _ = stop.changed() => return Ok(StreamEnd::Stopped),
        }
    }
}

// ---------------------------------------------------------------- consumer

async fn consume<St: Store, S: AccountSource>(
    config: IngestConfig,
    store: Arc<St>,
    idls: Arc<IdlCache<S>>,
    mut events: mpsc::Receiver<Event>,
) -> Result<IngestStats, IngestError> {
    let mut stats = IngestStats::default();
    let mut cursor: Option<u64> = None;
    let mut saved: Option<u64> = None;
    let mut last_flush = Instant::now();

    while let Some(event) = events.recv().await {
        match event {
            Event::Tx { slot, info } => match tx_input(slot, &info) {
                Ok(Some(input)) => {
                    let (failure, refreshed) = decode_with_idls(&input, &idls).await;
                    if let Some(program) = refreshed {
                        persist_idl(store.as_ref(), &idls, &program).await?;
                    }
                    if store.insert_failure(&failure).await? {
                        stats.inserted += 1;
                        tracing::debug!(
                            signature = %failure.signature,
                            program = ?failure.failing_program_id,
                            error = ?failure.error_name,
                            source = ?failure.decode_source,
                            "stored failure"
                        );
                    } else {
                        stats.duplicates += 1;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    stats.convert_errors += 1;
                    tracing::warn!(slot, error = %e, "could not convert transaction");
                }
            },
            Event::BlockMeta { slot, block_time } => {
                if let Some(t) = block_time {
                    store.set_block_time(slot, t).await?;
                }
                cursor = Some(cursor.map_or(slot, |c| c.max(slot)));
            }
        }

        if last_flush.elapsed() >= config.cursor_flush_interval {
            flush_cursor(store.as_ref(), &config.stream, cursor, &mut saved).await?;
            last_flush = Instant::now();
            tracing::info!(?cursor, ?stats, "progress");
        }
    }
    flush_cursor(store.as_ref(), &config.stream, cursor, &mut saved).await?;
    tracing::info!(?cursor, ?stats, "consumer drained");
    Ok(stats)
}

async fn flush_cursor<St: Store>(
    store: &St,
    stream: &str,
    cursor: Option<u64>,
    saved: &mut Option<u64>,
) -> Result<(), IngestError> {
    if let Some(slot) = cursor {
        if *saved != Some(slot) {
            store.save_cursor(stream, slot).await?;
            *saved = Some(slot);
        }
    }
    Ok(())
}

/// Decodes; if a custom code is still unnamed, fetches the failing
/// program's IDL (cached, including "no IDL") and decodes again.
/// Also returns the program whose cache entry was just refreshed, if any,
/// so the caller can persist it.
pub async fn decode_with_idls<S: AccountSource>(
    input: &TxInput,
    idls: &IdlCache<S>,
) -> (DecodedFailure, Option<String>) {
    let first = decode(input, idls);
    let needs_idl = first.decode_source == DecodeSource::Unknown && first.error_code.is_some();
    match (&first.failing_program_id, needs_idl) {
        (Some(program), true) => {
            let ensured = idls.ensure(program).await;
            let refreshed = ensured.refreshed.then(|| program.clone());
            (decode(input, idls), refreshed)
        }
        _ => (first, None),
    }
}

/// Loads persisted IDL lookups into the cache, keeping their original age.
async fn seed_idls<St: Store, S: AccountSource>(
    store: &St,
    idls: &IdlCache<S>,
) -> Result<usize, IngestError> {
    let rows = store.load_idls().await?;
    let mut seeded = 0;
    for row in rows {
        let status = match row.fetch_status.as_str() {
            "found" => FetchStatus::Found,
            "missing" => FetchStatus::Missing,
            _ => continue,
        };
        let idl = match (
            status,
            row.idl_json,
            row.location.as_deref().and_then(IdlLocation::parse),
        ) {
            (FetchStatus::Found, Some(json), Some(location)) => Some(Idl {
                program_id: row.program_id.clone(),
                location,
                format: row
                    .idl_format
                    .as_deref()
                    .and_then(IdlFormat::parse)
                    .unwrap_or_else(|| detect_format(&json)),
                json,
            }),
            (FetchStatus::Found, _, _) => continue,
            _ => None,
        };
        idls.seed(
            &row.program_id,
            status,
            idl,
            Duration::from_secs(row.age_secs),
        );
        seeded += 1;
    }
    Ok(seeded)
}

/// Writes a program's current cache entry (found or missing) to the store.
async fn persist_idl<St: Store, S: AccountSource>(
    store: &St,
    idls: &IdlCache<S>,
    program_id: &str,
) -> Result<(), IngestError> {
    let Some(entry) = idls.entry(program_id) else {
        return Ok(());
    };
    let fetch_status = match entry.status {
        FetchStatus::Found => "found",
        FetchStatus::Missing => "missing",
        FetchStatus::Error => return Ok(()),
    };
    store
        .upsert_idl(&IdlCacheRow {
            program_id: program_id.to_string(),
            fetch_status: fetch_status.to_string(),
            location: entry.idl.as_ref().map(|i| i.location.as_str().to_string()),
            idl_format: entry.idl.as_ref().map(|i| i.format.to_text()),
            idl_json: entry.idl.as_ref().map(|i| i.json.clone()),
            age_secs: entry.fetched_at.elapsed().as_secs(),
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yellowstone_grpc_proto::tonic::Code;

    #[test]
    fn recognizes_replay_refusals() {
        let mut replay = ReplayState::default();
        let lagged = Status::new(
            Code::Internal,
            "broadcast from 100 is not available, last available: 250",
        );
        assert!(replay_refused(&lagged, &mut replay));
        assert_eq!(replay.oldest, Some(250));

        let unsupported = Status::new(Code::Internal, "from_slot is not supported");
        assert!(replay_refused(&unsupported, &mut replay));
        assert!(replay.unsupported);

        let other = Status::new(Code::Unavailable, "connection reset");
        assert!(!replay_refused(&other, &mut ReplayState::default()));
    }
}
