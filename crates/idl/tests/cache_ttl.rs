//! Cache behaviour: TTLs per outcome, negative caching, stale-on-error.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use failscope_decoder::ErrorLookup;
use failscope_idl::{
    AccountData, AccountSource, CacheTtl, FetchStatus, IdlCache, IdlError, StaticAccounts,
};

/// Wraps recorded accounts, counts calls, and can be switched to fail.
#[derive(Clone)]
struct Flaky {
    inner: StaticAccounts,
    calls: Arc<AtomicUsize>,
    failing: Arc<std::sync::atomic::AtomicBool>,
}

impl AccountSource for Flaky {
    async fn get_accounts(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<AccountData>>, IdlError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(IdlError::Rpc("down".to_string()));
        }
        self.inner.get_accounts(addresses).await
    }
}

fn setup() -> (IdlCache<Flaky>, Flaky) {
    let source = Flaky {
        inner: recorded_accounts(),
        calls: Arc::default(),
        failing: Arc::default(),
    };
    let ttl = CacheTtl {
        found: Duration::from_secs(600),
        missing: Duration::from_secs(60),
        error: Duration::from_secs(10),
    };
    (IdlCache::with_ttl(source.clone(), ttl), source)
}

#[tokio::test(start_paused = true)]
async fn found_is_cached_until_ttl() {
    let (cache, source) = setup();
    assert_eq!(cache.ensure(FAIL_TARGET).await, FetchStatus::Found);
    assert_eq!(cache.ensure(FAIL_TARGET).await, FetchStatus::Found);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(601)).await;
    cache.ensure(FAIL_TARGET).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn missing_is_cached_with_its_own_ttl() {
    let (cache, source) = setup();
    assert_eq!(cache.ensure(NO_IDL).await, FetchStatus::Missing);
    cache.ensure(NO_IDL).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(61)).await;
    cache.ensure(NO_IDL).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    assert!(!cache.has_idl(NO_IDL));
}

#[tokio::test(start_paused = true)]
async fn errors_retry_soon_and_never_drop_a_known_idl() {
    let (cache, source) = setup();
    source.failing.store(true, Ordering::SeqCst);
    assert_eq!(cache.ensure(FAIL_TARGET).await, FetchStatus::Error);
    cache.ensure(FAIL_TARGET).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(11)).await;

    source.failing.store(false, Ordering::SeqCst);
    assert_eq!(cache.ensure(FAIL_TARGET).await, FetchStatus::Found);
    assert!(cache.idl_error(FAIL_TARGET, 6000).is_some());

    // Refresh fails after expiry: the found IDL keeps serving, and the next
    // retry comes after the error TTL rather than the found TTL.
    tokio::time::advance(Duration::from_secs(601)).await;
    source.failing.store(true, Ordering::SeqCst);
    assert_eq!(cache.ensure(FAIL_TARGET).await, FetchStatus::Found);
    assert!(cache.idl_error(FAIL_TARGET, 6000).is_some());
    let calls = source.calls.load(Ordering::SeqCst);
    cache.ensure(FAIL_TARGET).await;
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        calls,
        "retried too soon"
    );
    tokio::time::advance(Duration::from_secs(11)).await;
    cache.ensure(FAIL_TARGET).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), calls + 1);
}
