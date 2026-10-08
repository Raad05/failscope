//! IDL cache with per-outcome TTLs, including negative results.
//!
//! The cache implements the decoder's [`ErrorLookup`], so callers `ensure`
//! the programs they need (async, may hit the network) and then decode
//! synchronously against the cache. Stale entries keep serving until a
//! refresh succeeds or fails.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use failscope_decoder::{ErrorLookup, IdlError as DecodedIdlError, IdlErrorTable};
use serde::Serialize;
use tokio::time::Instant;

use crate::fetch::{fetch_idl, Idl};
use crate::source::AccountSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatus {
    Found,
    /// No IDL in either location.
    Missing,
    /// The fetch or decode failed; retried sooner than `Missing`.
    Error,
}

#[derive(Debug, Clone, Copy)]
pub struct CacheTtl {
    pub found: Duration,
    pub missing: Duration,
    pub error: Duration,
}

impl Default for CacheTtl {
    fn default() -> Self {
        Self {
            found: Duration::from_secs(6 * 3600),
            missing: Duration::from_secs(3600),
            error: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: FetchStatus,
    pub idl: Option<Arc<Idl>>,
    pub fetched_at: Instant,
    errors: IdlErrorTable,
}

pub struct IdlCache<S> {
    source: S,
    ttl: CacheTtl,
    entries: RwLock<HashMap<String, CacheEntry>>,
}

impl<S: AccountSource> IdlCache<S> {
    pub fn new(source: S) -> Self {
        Self::with_ttl(source, CacheTtl::default())
    }

    pub fn with_ttl(source: S, ttl: CacheTtl) -> Self {
        Self {
            source,
            ttl,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Makes sure `program_id` has a fresh entry, fetching if needed.
    /// Concurrent calls for the same program may both fetch; the last wins.
    pub async fn ensure(&self, program_id: &str) -> FetchStatus {
        if let Some(entry) = self.entry(program_id) {
            if entry.fetched_at.elapsed() < self.ttl_for(entry.status) {
                return entry.status;
            }
        }

        let (status, idl) = match fetch_idl(&self.source, program_id).await {
            Ok(Some(idl)) => (FetchStatus::Found, Some(Arc::new(idl))),
            Ok(None) => (FetchStatus::Missing, None),
            Err(e) => {
                tracing::warn!(program_id, error = %e, "IDL fetch failed");
                (FetchStatus::Error, None)
            }
        };

        let mut errors = IdlErrorTable::new();
        if let Some(idl) = &idl {
            if let Err(e) = errors.add_idl(program_id, &idl.json) {
                tracing::warn!(program_id, error = %e, "IDL has an unreadable errors array");
            }
        }

        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        // A failed refresh keeps serving the previously found IDL.
        let keep_previous = status == FetchStatus::Error
            && entries
                .get(program_id)
                .is_some_and(|e| e.status == FetchStatus::Found);
        if keep_previous {
            if let Some(previous) = entries.get_mut(program_id) {
                previous.fetched_at = Instant::now()
                    .checked_sub(self.ttl.found.saturating_sub(self.ttl.error))
                    .unwrap_or_else(Instant::now);
            }
            return FetchStatus::Found;
        }
        entries.insert(
            program_id.to_string(),
            CacheEntry {
                status,
                idl,
                fetched_at: Instant::now(),
                errors,
            },
        );
        status
    }

    pub fn entry(&self, program_id: &str) -> Option<CacheEntry> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(program_id)
            .cloned()
    }

    fn ttl_for(&self, status: FetchStatus) -> Duration {
        match status {
            FetchStatus::Found => self.ttl.found,
            FetchStatus::Missing => self.ttl.missing,
            FetchStatus::Error => self.ttl.error,
        }
    }
}

impl<S: AccountSource> ErrorLookup for IdlCache<S> {
    fn idl_error(&self, program_id: &str, code: u32) -> Option<DecodedIdlError> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(program_id)?
            .errors
            .idl_error(program_id, code)
    }

    fn has_idl(&self, program_id: &str) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(program_id)
            .is_some_and(|e| e.idl.is_some())
    }
}
