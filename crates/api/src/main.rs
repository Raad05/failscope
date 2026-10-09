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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A missing .env is fine; real deployments set the environment directly.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        // Plain text when logging to a file or pipe.
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
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
