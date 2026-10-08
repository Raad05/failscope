//! Real fixtures with their logs damaged, to check behaviour when logs can't
//! be trusted: truncated, stripped of Anchor lines, or missing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use failscope_decoder::{decode, Confidence, DecodeSource, NoIdls, TxInput};

const FAIL_TARGET: &str = "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey";

/// Keeps logs up to (not including) the first `failed:` line, then appends
/// the runtime's truncation marker.
fn truncate_before_failure(mut tx: TxInput) -> TxInput {
    let logs = tx.logs.take().unwrap();
    let cut = logs.iter().position(|l| l.contains(" failed: ")).unwrap();
    let mut kept: Vec<String> = logs.into_iter().take(cut).collect();
    kept.push("Log truncated".to_string());
    tx.logs = Some(kept);
    tx
}

#[test]
fn truncated_without_cpi_still_attributes_and_decodes_via_idl() {
    // No CPI ran, so the top-level program must be the one that failed. With
    // the AnchorError line gone, the IDL supplies the name.
    let tx = truncate_before_failure(common::fixture("devnet_custom_error"));
    let d = decode(&tx, &common::fixture_idls());
    assert!(d.log_truncated);
    assert_eq!(d.failing_program_id.as_deref(), Some(FAIL_TARGET));
    assert_eq!(d.cpi_depth, Some(1));
    assert_eq!(d.attribution_confidence, Confidence::High);
    assert_eq!(d.decode_source, DecodeSource::Idl);
    assert_eq!(d.error_name.as_deref(), Some("AlwaysFails"));
}

#[test]
fn truncated_with_cpi_is_low_confidence_and_not_named() {
    // fail_target CPIs fail_callee. Both define 6000, so guessing the program
    // would also guess the meaning. The decoder must not pretend.
    let tx = truncate_before_failure(common::fixture("devnet_nested_cpi_callee"));
    let d = decode(&tx, &common::fixture_idls());
    assert!(d.log_truncated);
    assert_eq!(d.attribution_confidence, Confidence::Low);
    assert_eq!(d.failing_program_id.as_deref(), Some(FAIL_TARGET));
    assert_eq!(d.cpi_depth, None);
    assert_eq!(d.error_code, Some(6000));
    assert_eq!(d.error_name, None);
    assert_eq!(d.decode_source, DecodeSource::Unknown);
}

#[test]
fn truncated_runtime_error_is_still_named_by_variant() {
    let tx = truncate_before_failure(common::fixture("mainnet_jupiter_panic"));
    let d = decode(&tx, &NoIdls);
    assert_eq!(d.attribution_confidence, Confidence::Low);
    assert_eq!(d.decode_source, DecodeSource::Runtime);
    // Without the `failed:` reason, panic vs CU exhaustion can't be told apart.
    assert_eq!(d.error_name.as_deref(), Some("ProgramFailedToComplete"));
}

#[test]
fn anchor_lines_stripped_falls_back_to_idl_then_framework() {
    let strip = |name: &str| {
        let mut tx = common::fixture(name);
        tx.logs = tx.logs.map(|l| {
            l.into_iter()
                .filter(|l| !l.contains("AnchorError"))
                .collect()
        });
        tx
    };
    let idls = common::fixture_idls();

    let custom = decode(&strip("devnet_custom_error"), &idls);
    assert_eq!(custom.decode_source, DecodeSource::Idl);
    assert_eq!(custom.error_name.as_deref(), Some("AlwaysFails"));
    assert_eq!(custom.attribution_confidence, Confidence::High);

    let nested = decode(&strip("devnet_nested_cpi_callee"), &idls);
    assert_eq!(nested.decode_source, DecodeSource::Idl);
    assert_eq!(nested.error_name.as_deref(), Some("CalleeAlwaysFails"));

    let has_one = decode(&strip("devnet_has_one_violation"), &idls);
    assert_eq!(has_one.decode_source, DecodeSource::AnchorFramework);
    assert_eq!(has_one.error_name.as_deref(), Some("ConstraintHasOne"));

    // Without an IDL the program isn't known to be Anchor: no framework guess.
    let no_idl = decode(&strip("devnet_has_one_violation"), &NoIdls);
    assert_eq!(no_idl.decode_source, DecodeSource::Unknown);
}

#[test]
fn missing_logs_entirely() {
    let mut tx = common::fixture("devnet_cpi_system_transfer");
    tx.logs = None;
    let d = decode(&tx, &NoIdls);
    assert!(!d.log_truncated);
    assert_eq!(d.attribution_confidence, Confidence::Low);
    assert_eq!(d.decode_source, DecodeSource::Unknown);
}
