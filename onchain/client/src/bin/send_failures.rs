//! Sends every failing transaction to devnet with `skip_preflight` (otherwise
//! simulation rejects them and they never land), waits for each to confirm,
//! and writes the signatures to `fixtures/signatures.json`.
//!
//! Usage: `cargo run -p fail_client --bin send_failures [-- <case slug>...]`
//! Env: `RPC_URL` (default devnet), `KEYPAIR` (default ~/.config/solana/id.json).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use fail_client::{build, counter_pda, initialize_ix, Case};
use serde::Serialize;
use solana_commitment_config::CommitmentConfig;
use solana_keypair::{read_keypair_file, Keypair};
use solana_message::Message;
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSendTransactionConfig;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Serialize)]
struct Sent {
    case: &'static str,
    signature: String,
    cluster: String,
    err: String,
}

fn main() -> Result<()> {
    let url = std::env::var("RPC_URL").unwrap_or_else(|_| DEFAULT_RPC.to_string());
    if url.contains("mainnet") {
        bail!("refusing to send deliberately failing transactions to mainnet ({url})");
    }
    let keypair_path = match std::env::var("KEYPAIR") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(std::env::var("HOME")?).join(".config/solana/id.json"),
    };
    let payer = read_keypair_file(&keypair_path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", keypair_path.display()))?;

    let only: Vec<String> = std::env::args().skip(1).collect();
    let cases: Vec<Case> = Case::ALL
        .into_iter()
        .filter(|c| only.is_empty() || only.iter().any(|s| s == c.slug()))
        .collect();
    if cases.is_empty() {
        bail!("no case matches {only:?}");
    }

    let rpc = RpcClient::new_with_commitment(url.clone(), CommitmentConfig::confirmed());
    println!("payer {} on {url}", payer.pubkey());

    ensure_counter(&rpc, &payer)?;

    let mut sent = Vec::new();
    for case in cases {
        let built = build(case, &payer.pubkey());
        let mut signers: Vec<&Keypair> = vec![&payer];
        signers.extend(built.extra_signers.iter());
        let tx = Transaction::new(
            &signers,
            Message::new(&built.instructions, Some(&payer.pubkey())),
            rpc.get_latest_blockhash()?,
        );
        let signature = rpc
            .send_transaction_with_config(
                &tx,
                RpcSendTransactionConfig {
                    skip_preflight: true,
                    ..Default::default()
                },
            )
            .with_context(|| format!("sending {}", case.slug()))?;

        let err = match wait_for_status(&rpc, &signature)? {
            Some(Err(e)) => format!("{e:?}"),
            Some(Ok(())) => bail!("{} unexpectedly succeeded: {signature}", case.slug()),
            None => bail!(
                "{} not confirmed within {CONFIRM_TIMEOUT:?}: {signature}",
                case.slug()
            ),
        };
        println!("{:<22} {signature}  {err}", case.slug());
        sent.push(Sent {
            case: case.slug(),
            signature: signature.to_string(),
            cluster: url.clone(),
            err,
        });
    }

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/signatures.json");
    std::fs::create_dir_all(out.parent().context("fixtures path has no parent")?)?;
    std::fs::write(&out, serde_json::to_string_pretty(&sent)? + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

/// The has_one case needs the counter PDA to exist, owned by the payer.
fn ensure_counter(rpc: &RpcClient, payer: &Keypair) -> Result<()> {
    let pda = counter_pda();
    if rpc
        .get_account_with_commitment(&pda, CommitmentConfig::confirmed())?
        .value
        .is_some()
    {
        return Ok(());
    }
    let tx = Transaction::new(
        &[payer],
        Message::new(&[initialize_ix(&payer.pubkey())], Some(&payer.pubkey())),
        rpc.get_latest_blockhash()?,
    );
    let sig = rpc
        .send_and_confirm_transaction(&tx)
        .context("initializing counter")?;
    println!("initialized counter {pda}: {sig}");
    Ok(())
}

/// Polls until the signature has a confirmed status or the timeout passes.
fn wait_for_status(
    rpc: &RpcClient,
    signature: &Signature,
) -> Result<Option<Result<(), TransactionError>>> {
    let start = Instant::now();
    while start.elapsed() < CONFIRM_TIMEOUT {
        if let Some(status) = rpc.get_signature_status(signature)? {
            return Ok(Some(status));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(None)
}
