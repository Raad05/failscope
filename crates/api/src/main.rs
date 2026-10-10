use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use failscope_idl::{IdlCache, RpcAccountSource};
use failscope_ingest::IngestConfig;
use failscope_store::PgStore;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Solana failed-transaction analyzer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Stream failed transactions from Yellowstone gRPC, decode and store them.
    Ingest {
        /// Yellowstone gRPC endpoint, e.g. http://127.0.0.1:10000
        #[arg(long, env = "YELLOWSTONE_ENDPOINT")]
        endpoint: String,
        /// x-token for hosted endpoints.
        #[arg(long, env = "YELLOWSTONE_X_TOKEN", hide_env_values = true)]
        x_token: Option<String>,
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        /// JSON-RPC endpoint used to fetch IDLs.
        #[arg(long, env = "RPC_URL")]
        rpc_url: String,
        /// Cursor name, so several streams can share one database.
        #[arg(long, env = "INGEST_STREAM", default_value = "default")]
        stream: String,
    },
    /// Watch per-program failure rates and POST a webhook when one spikes.
    Alert {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        /// Receives JSON alerts (Slack-compatible `text` field included). Secret.
        #[arg(long, env = "ALERT_WEBHOOK_URL", hide_env_values = true)]
        webhook_url: String,
        /// Minutes counted as "now".
        #[arg(long, env = "ALERT_WINDOW_MINUTES", default_value_t = 5)]
        window_minutes: u32,
        /// Minutes before the window used as the program's normal rate.
        #[arg(long, env = "ALERT_BASELINE_MINUTES", default_value_t = 60)]
        baseline_minutes: u32,
        /// Fire when the window count reaches this multiple of the baseline.
        #[arg(long, env = "ALERT_FACTOR", default_value_t = 3.0)]
        factor: f64,
        /// Never fire below this many failures in the window.
        #[arg(long, env = "ALERT_MIN_FAILURES", default_value_t = 10)]
        min_failures: u64,
        #[arg(long, env = "ALERT_INTERVAL_SECS", default_value_t = 60)]
        interval_secs: u64,
        /// Programs to watch (repeat or comma-separate); default all.
        #[arg(long = "program", env = "ALERT_PROGRAMS", value_delimiter = ',')]
        programs: Vec<String>,
        /// Dashboard base URL, linked from alerts.
        #[arg(long, env = "DASHBOARD_URL")]
        dashboard_url: Option<String>,
    },
    /// Serve the JSON API.
    Serve {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, env = "BIND", default_value = "127.0.0.1:8080")]
        bind: std::net::SocketAddr,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A missing .env is fine; real deployments set the environment directly.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        // Plain text when logging to a file or pipe.
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .init();

    match Cli::parse().command {
        Command::Ingest {
            endpoint,
            x_token,
            database_url,
            rpc_url,
            stream,
        } => {
            let store = PgStore::connect(&database_url)
                .await
                .context("connecting to Postgres")?;
            let idls = IdlCache::new(RpcAccountSource::new(rpc_url));
            let mut config = IngestConfig::new(endpoint);
            config.x_token = x_token.filter(|t| !t.is_empty());
            config.stream = stream;

            let stats =
                failscope_ingest::run(config, Arc::new(store), Arc::new(idls), shutdown_signal())
                    .await?;
            tracing::info!(?stats, "ingest stopped");
        }
        Command::Alert {
            database_url,
            webhook_url,
            window_minutes,
            baseline_minutes,
            factor,
            min_failures,
            interval_secs,
            programs,
            dashboard_url,
        } => {
            anyhow::ensure!(window_minutes >= 1, "--window-minutes must be at least 1");
            anyhow::ensure!(
                baseline_minutes >= window_minutes,
                "--baseline-minutes must be at least --window-minutes"
            );
            anyhow::ensure!(factor >= 1.0, "--factor must be at least 1");
            anyhow::ensure!(interval_secs >= 5, "--interval-secs must be at least 5");
            let store = PgStore::connect(&database_url)
                .await
                .context("connecting to Postgres")?;
            let config = failscope_alert::AlertConfig {
                rule: failscope_alert::Rule {
                    window_minutes,
                    baseline_minutes,
                    factor,
                    min_failures,
                    ..Default::default()
                },
                webhook_url,
                interval: std::time::Duration::from_secs(interval_secs),
                programs: programs.into_iter().filter(|p| !p.is_empty()).collect(),
                dashboard_url,
            };
            failscope_alert::run(config, Arc::new(store), shutdown_signal()).await?;
        }
        Command::Serve { database_url, bind } => {
            let store = PgStore::connect(&database_url)
                .await
                .context("connecting to Postgres")?;
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            tracing::info!(%bind, "serving API");
            axum::serve(listener, failscope_api::http::router(Arc::new(store)))
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }
    Ok(())
}

/// Resolves on Ctrl-C (SIGINT) or SIGTERM, which is what Docker and systemd send.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "could not listen for ctrl-c");
            std::future::pending::<()>().await;
        }
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
