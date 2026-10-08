//! Runs every failure case against the real compiled programs in LiteSVM.
//! Requires `anchor build` first (reads target/deploy/*.so).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use fail_client::{build, initialize_ix, Case};
use litesvm::types::FailedTransactionMetadata;
use litesvm::LiteSVM;
use solana_instruction_error::InstructionError;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

fn so_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/deploy")
        .join(format!("{name}.so"))
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    for (id, name) in [
        (fail_target::ID, "fail_target"),
        (fail_callee::ID, "fail_callee"),
    ] {
        let path = so_path(name);
        assert!(
            path.exists(),
            "{} missing: run `anchor build` in onchain/ first",
            path.display()
        );
        svm.add_program_from_file(id, path).unwrap();
    }
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
    let tx = Transaction::new(
        &[&payer],
        Message::new(&[initialize_ix(&payer.pubkey())], Some(&payer.pubkey())),
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();
    (svm, payer)
}

fn run(case: Case) -> FailedTransactionMetadata {
    let (mut svm, payer) = setup();
    let built = build(case, &payer.pubkey());
    let mut signers: Vec<&Keypair> = vec![&payer];
    signers.extend(built.extra_signers.iter());
    let tx = Transaction::new(
        &signers,
        Message::new(&built.instructions, Some(&payer.pubkey())),
        svm.latest_blockhash(),
    );
    let failed = match svm.send_transaction(tx) {
        Ok(meta) => panic!("{case:?} succeeded:\n{}", meta.pretty_logs()),
        Err(failed) => failed,
    };
    println!(
        "== {} ==\nerr: {:?}\n{}",
        case.slug(),
        failed.err,
        failed.meta.pretty_logs()
    );
    failed
}

fn assert_failed_line(failed: &FailedTransactionMetadata, program: &str, reason: &str) {
    let first = failed
        .meta
        .logs
        .iter()
        .find(|l| l.contains(" failed: "))
        .unwrap_or_else(|| panic!("no `failed:` line in {:#?}", failed.meta.logs));
    assert!(
        first.starts_with(&format!("Program {program} failed: ")) && first.contains(reason),
        "innermost failure line was {first:?}"
    );
}

fn ix_err(index: u8, err: InstructionError) -> TransactionError {
    TransactionError::InstructionError(index, err)
}

const TARGET: &str = "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey";
const CALLEE: &str = "BCUANLyTtzvYGheyo7ymmHhDDZQ74WGjwsC8Rv3VFDPb";
const SYSTEM: &str = "11111111111111111111111111111111";

#[test]
fn custom_error() {
    let f = run(Case::Custom);
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(6000)));
    assert_failed_line(&f, TARGET, "custom program error: 0x1770");
    assert!(f
        .meta
        .logs
        .iter()
        .any(|l| l.contains("AnchorError") && l.contains("AlwaysFails") && l.contains("6000")));
}

#[test]
fn has_one_violation() {
    let f = run(Case::HasOne);
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(2001)));
    assert_failed_line(&f, TARGET, "custom program error: 0x7d1");
}

#[test]
fn missing_signer() {
    let f = run(Case::MissingSigner);
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(3010)));
    assert_failed_line(&f, TARGET, "custom program error: 0xbc2");
}

#[test]
fn overflow_panic() {
    let f = run(Case::OverflowPanic);
    assert_eq!(f.err, ix_err(0, InstructionError::ProgramFailedToComplete));
    assert_failed_line(&f, TARGET, "SBF program Panicked in");
    assert!(f
        .meta
        .logs
        .iter()
        .any(|l| l.contains("attempt to add with overflow")));
}

#[test]
fn overflow_checked() {
    let f = run(Case::OverflowChecked);
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(6001)));
    assert_failed_line(&f, TARGET, "custom program error: 0x1771");
}

#[test]
fn compute_exhausted() {
    let f = run(Case::Compute);
    // Index 1: the compute-budget instruction comes first. Running out of CUs
    // is reported as ProgramFailedToComplete, the same variant as a panic;
    // only the `failed:` log line tells the two apart.
    assert_eq!(f.err, ix_err(1, InstructionError::ProgramFailedToComplete));
    assert_failed_line(&f, TARGET, "exceeded CUs meter at BPF instruction");
}

#[test]
fn cpi_system_transfer() {
    let f = run(Case::CpiSystem);
    // Top-level error points at fail_target's instruction, but the program
    // that failed is the System program.
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(1)));
    assert_failed_line(&f, SYSTEM, "custom program error: 0x1");
}

#[test]
fn nested_cpi_callee() {
    let f = run(Case::NestedCpi);
    // Same code as `custom_error`, different program: decoding must key on
    // (program_id, code).
    assert_eq!(f.err, ix_err(0, InstructionError::Custom(6000)));
    assert_failed_line(&f, CALLEE, "custom program error: 0x1770");
    assert!(f.meta.logs.iter().any(|l| l.contains("CalleeAlwaysFails")));
}
