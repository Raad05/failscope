//! Alerting end to end against a real Postgres and a fake webhook receiver.
//! Needs `DATABASE_URL`; skips without it unless `FAILSCOPE_REQUIRE_DB=1`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use failscope_alert::{tick, AlertConfig, AlertMetric, Rule};
use failscope_decoder::{Confidence, DecodeSource, DecodedFailure};
use failscope_score::BotClass;
use failscope_store::{AlertState, AlertStore, PgStore, Store};

const SPIKY: &str = "SpikyProgram1111111111111111111111111111111";
const STEADY: &str = "SteadyProgram111111111111111111111111111111";

#[derive(Clone, Default)]
struct Receiver {
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
    failing: Arc<AtomicBool>,
}

async fn receive(State(r): State<Receiver>, Json(body): Json<serde_json::Value>) -> StatusCode {
    if r.failing.load(Ordering::SeqCst) {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    r.payloads.lock().unwrap().push(body);
    StatusCode::NO_CONTENT
}

async fn start_receiver() -> (Receiver, String) {
    let receiver = Receiver::default();
    let app = Router::new()
        .route("/hook", post(receive))
        .with_state(receiver.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (receiver, url)
}

async fn fresh_store() -> Option<(PgStore, PgStore, String)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) if std::env::var("FAILSCOPE_REQUIRE_DB").as_deref() == Ok("1") => {
            panic!("FAILSCOPE_REQUIRE_DB=1 but DATABASE_URL is not set")
        }
        Err(_) => return None,
    };
    let admin = PgStore::connect(&url).await.unwrap();
    let name = format!(
        "failscope_alert_test_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(admin.pool())
        .await
        .unwrap();
    let (base, _) = url.rsplit_once('/').unwrap();
    let store = PgStore::connect(&format!("{base}/{name}")).await.unwrap();
    Some((admin, store, name))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// `per_minute` failures for `program` in each minute of ages `ages`
/// (age 0 = the current minute).
async fn seed(store: &PgStore, program: &str, ages: std::ops::Range<i64>, per_minute: usize) {
    for age in ages {
        for k in 0..per_minute {
            let f = DecodedFailure {
                signature: format!("{program}-{age}-{k}"),
                slot: 1,
                block_time: Some(now() - age * 60 - 10),
                fee: 5000,
                priority_fee: 0,
                cu_requested: 200_000,
                cu_consumed: None,
                cu_price: 0,
                cu_limit_explicit: false,
                jito_tip: false,
                signer: None,
                uses_alt: false,
                log_truncated: false,
                top_level_ix_index: Some(0),
                root_program_id: Some(program.to_string()),
                failing_program_id: Some(program.to_string()),
                cpi_depth: Some(1),
                attribution_confidence: Confidence::High,
                error_kind: "custom".to_string(),
                error_code: Some(6001),
                error_name: Some("SlippageToleranceExceeded".to_string()),
                error_message: None,
                error_account: None,
                decode_source: DecodeSource::Idl,
                raw_err: serde_json::json!({}),
            };
            store.insert_failure(&f, None).await.unwrap();
        }
    }
}

#[tokio::test]
async fn fires_once_retries_failed_delivery_and_resolves() {
    let Some((admin, store, name)) = fresh_store().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    // Spiky: 2/min for the hour before, 8/min in the last 5 minutes.
    seed(&store, SPIKY, 5..65, 2).await;
    seed(&store, SPIKY, 0..5, 8).await;
    // Steady: 4/min throughout. Busy, but normal for it.
    seed(&store, STEADY, 0..65, 4).await;

    let (receiver, url) = start_receiver().await;
    let config = AlertConfig {
        rule: Rule::default(),
        metric: AlertMetric::Failures,
        webhook_url: url,
        interval: Duration::from_secs(60),
        programs: vec![],
        dashboard_url: Some("http://127.0.0.1:8080/".to_string()),
    };
    let client = reqwest::Client::new();

    // Webhook down: the transition is not recorded.
    receiver.failing.store(true, Ordering::SeqCst);
    assert!(tick(&config, &store, &client).await.is_err());
    assert!(
        store
            .alert_states(AlertMetric::Failures)
            .await
            .unwrap()
            .is_empty(),
        "recorded an undelivered alert"
    );

    // Webhook up: fires for the spiky program only.
    receiver.failing.store(false, Ordering::SeqCst);
    let events = tick(&config, &store, &client).await.unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].program_id, SPIKY);
    assert_eq!(events[0].state, AlertState::Firing);
    assert_eq!(events[0].current_count, 40);
    assert!((events[0].threshold - 30.0).abs() < 1e-9);
    {
        let payloads = receiver.payloads.lock().unwrap();
        assert_eq!(payloads.len(), 1);
        let p = &payloads[0];
        assert_eq!(p["state"], "firing");
        assert_eq!(p["program_id"], SPIKY);
        assert_eq!(
            p["top_errors"][0]["error_name"],
            "SlippageToleranceExceeded"
        );
        assert_eq!(
            p["dashboard_url"],
            format!("http://127.0.0.1:8080/#hours=1&program={SPIKY}")
        );
        assert!(p["text"].as_str().unwrap().contains("spiking"));
    }

    // Still spiking: no duplicate.
    assert!(tick(&config, &store, &client).await.unwrap().is_empty());

    // Burst over: drop the last 5 minutes and it resolves.
    sqlx::query("DELETE FROM failed_tx WHERE failing_program_id = $1 AND block_time > now() - interval '5 minutes'")
        .bind(SPIKY)
        .execute(store.pool())
        .await
        .unwrap();
    let events = tick(&config, &store, &client).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].state, AlertState::Resolved);
    assert_eq!(receiver.payloads.lock().unwrap()[1]["state"], "resolved");
    assert_eq!(
        store.alert_states(AlertMetric::Failures).await.unwrap()[SPIKY],
        AlertState::Resolved
    );
    assert_eq!(store.alert_events(1, 10).await.unwrap().len(), 2);

    // A program filter excludes everything else.
    seed(&store, SPIKY, 0..5, 8).await;
    let only_steady = AlertConfig {
        programs: vec![STEADY.to_string()],
        ..config.clone()
    };
    assert!(tick(&only_steady, &store, &client)
        .await
        .unwrap()
        .is_empty());

    drop(store);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(admin.pool())
        .await
        .unwrap();
}

const HUMANS: &str = "HumansProgram111111111111111111111111111111";
const BOTTY: &str = "BottyProgram1111111111111111111111111111111";

/// One failure per signer in the current minute, stored with `class`.
async fn seed_signers(store: &PgStore, program: &str, signers: &[String], class: BotClass) {
    for (k, signer) in signers.iter().enumerate() {
        let f = DecodedFailure {
            signature: format!("{program}-{signer}-{k}"),
            slot: 1,
            block_time: Some(now() - 10),
            fee: 5000,
            priority_fee: 0,
            cu_requested: 200_000,
            cu_consumed: None,
            cu_price: 0,
            cu_limit_explicit: false,
            jito_tip: false,
            signer: Some(signer.clone()),
            uses_alt: false,
            log_truncated: false,
            top_level_ix_index: Some(0),
            root_program_id: Some(program.to_string()),
            failing_program_id: Some(program.to_string()),
            cpi_depth: Some(1),
            attribution_confidence: Confidence::High,
            error_kind: "custom".to_string(),
            error_code: Some(6001),
            error_name: Some("SlippageToleranceExceeded".to_string()),
            error_message: None,
            error_account: None,
            decode_source: DecodeSource::Idl,
            raw_err: serde_json::json!({}),
        };
        let bot = failscope_score::BotScore {
            score: 0,
            class,
            reasons: vec![],
            features: failscope_score::Features {
                window_failures: 1,
                window_slots: 1_500,
                retry_run: 1,
                tipped: false,
                cu_price: 0,
                fee_percentile: None,
                silent_program: false,
            },
        };
        store.insert_failure(&f, Some(&bot)).await.unwrap();
    }
}

/// Counting likely-human signers: twelve people failing fires, one bot
/// failing 200 times doesn't (it would on the plain failure count).
#[tokio::test]
async fn human_signer_metric_ignores_a_single_noisy_bot() {
    let Some((admin, store, name)) = fresh_store().await else {
        return;
    };
    let people: Vec<String> = (0..12).map(|i| format!("person{i}")).collect();
    seed_signers(&store, HUMANS, &people, BotClass::LikelyHuman).await;
    let bot: Vec<String> = (0..200).map(|_| "bot".to_string()).collect();
    seed_signers(&store, BOTTY, &bot, BotClass::LikelyBot).await;

    let (receiver, url) = start_receiver().await;
    let humans = AlertConfig {
        rule: Rule::default(),
        metric: AlertMetric::HumanSigners,
        webhook_url: url,
        interval: Duration::from_secs(60),
        programs: vec![],
        dashboard_url: None,
    };
    let client = reqwest::Client::new();

    let fired = tick(&humans, &store, &client).await.unwrap();
    assert_eq!(
        fired
            .iter()
            .map(|e| e.program_id.as_str())
            .collect::<Vec<_>>(),
        [HUMANS]
    );
    assert_eq!(fired[0].current_count, 12);
    {
        let payloads = receiver.payloads.lock().unwrap();
        assert_eq!(payloads[0]["metric"], "human_signers");
        assert!(payloads[0]["text"]
            .as_str()
            .unwrap()
            .contains("likely-human wallet-minutes"));
    }

    // States are kept per metric: the failure count still alerts on the bot.
    let failures = AlertConfig {
        metric: AlertMetric::Failures,
        ..humans.clone()
    };
    let fired = tick(&failures, &store, &client).await.unwrap();
    let programs: Vec<&str> = fired.iter().map(|e| e.program_id.as_str()).collect();
    assert!(programs.contains(&BOTTY), "{programs:?}");
    assert_eq!(
        store.alert_states(AlertMetric::HumanSigners).await.unwrap()[HUMANS],
        AlertState::Firing
    );
    assert!(!store
        .alert_states(AlertMetric::HumanSigners)
        .await
        .unwrap()
        .contains_key(BOTTY));
    let events = store.alert_events(1, 10).await.unwrap();
    assert!(events
        .iter()
        .any(|e| e.program_id == HUMANS && e.metric == "human_signers"));

    drop(store);
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(admin.pool())
        .await
        .unwrap();
}
