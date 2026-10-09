//! IDL fetching and decoding against recorded on-chain accounts (no network).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use failscope_decoder::{decode, DecodeSource, ErrorLookup};
use failscope_idl::{fetch_idl, FetchStatus, IdlCache, IdlFormat, IdlLocation};

#[tokio::test]
async fn program_metadata_idl_from_devnet() {
    let idl = fetch_idl(&recorded_accounts(), FAIL_TARGET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idl.location, IdlLocation::ProgramMetadata);
    assert_eq!(idl.format, IdlFormat::Spec("0.1.0".to_string()));
    // Byte-for-byte the IDL `anchor build` produced.
    let built = read_json(
        &fixtures_dir()
            .join("idls")
            .join(format!("{FAIL_TARGET}.json")),
    );
    assert_eq!(idl.json, built);
}

#[tokio::test]
async fn legacy_location_with_spec_format_from_mainnet() {
    let idl = fetch_idl(&recorded_accounts(), JUPITER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idl.location, IdlLocation::Legacy);
    assert_eq!(idl.format, IdlFormat::Spec("0.1.0".to_string()));
}

#[tokio::test]
async fn legacy_location_with_legacy_format() {
    let idl = fetch_idl(&recorded_accounts(), MARINADE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idl.location, IdlLocation::Legacy);
    assert_eq!(idl.format, IdlFormat::Legacy);
    assert_eq!(idl.json["name"], "marinade_finance");

    // Legacy-format errors use the same shape, so the decoder reads them as is.
    let cache = IdlCache::new(recorded_accounts());
    assert_eq!(cache.ensure(MARINADE).await.status, FetchStatus::Found);
    let first = &idl.json["errors"][0];
    let code = u32::try_from(first["code"].as_u64().unwrap()).unwrap();
    let looked_up = cache.idl_error(MARINADE, code).unwrap();
    assert_eq!(looked_up.name, first["name"].as_str().unwrap());
}

#[tokio::test]
async fn program_without_idl_is_none() {
    assert_eq!(fetch_idl(&recorded_accounts(), NO_IDL).await.unwrap(), None);
}

/// M4 done-criterion: fail_target's errors decode from its on-chain IDL with
/// the AnchorError log line removed.
#[tokio::test]
async fn errors_decode_via_on_chain_idl_without_logs() {
    let cache = IdlCache::new(recorded_accounts());
    for program in [FAIL_TARGET, FAIL_CALLEE, JUPITER] {
        assert_eq!(
            cache.ensure(program).await.status,
            FetchStatus::Found,
            "{program}"
        );
    }
    assert_eq!(cache.ensure(NO_IDL).await.status, FetchStatus::Missing);

    let cases = [
        ("devnet_custom_error", DecodeSource::Idl, "AlwaysFails"),
        (
            "devnet_overflow_checked",
            DecodeSource::Idl,
            "CheckedOverflow",
        ),
        (
            "devnet_nested_cpi_callee",
            DecodeSource::Idl,
            "CalleeAlwaysFails",
        ),
        (
            "devnet_has_one_violation",
            DecodeSource::AnchorFramework,
            "ConstraintHasOne",
        ),
        (
            "devnet_missing_signer",
            DecodeSource::AnchorFramework,
            "AccountNotSigner",
        ),
        (
            "mainnet_jupiter_slippage",
            DecodeSource::Idl,
            "SlippageToleranceExceeded",
        ),
    ];
    for (fixture, source, name) in cases {
        let d = decode(&without_anchor_logs(fixture_tx(fixture)), &cache);
        assert_eq!(d.decode_source, source, "{fixture}");
        assert_eq!(d.error_name.as_deref(), Some(name), "{fixture}");
    }

    let unknown = decode(&fixture_tx("mainnet_cpi_inner_custom"), &cache);
    assert_eq!(unknown.decode_source, DecodeSource::Unknown);
}
