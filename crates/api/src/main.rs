mod config;

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // A missing .env is fine; real deployments set the environment directly.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = config::Config::from_env()?;
    tracing::info!(
        database = config.database_url.is_some(),
        rpc = config.rpc_url.is_some(),
        yellowstone = config.yellowstone_endpoint.is_some(),
        yellowstone_token = config.yellowstone_x_token.is_some(),
        "failscope starting (M0 skeleton, nothing to run yet)"
    );
    Ok(())
}
