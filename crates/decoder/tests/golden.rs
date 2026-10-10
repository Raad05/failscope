//! Every fixture under fixtures/txs must decode to its expected.json.
//!
//! All fields in expected.json are compared exactly, except `error_message`,
//! which is compared only when non-null (see fixtures/README.md). `description`
//! and `cluster` are documentation only.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use failscope_decoder::decode;

const DOC_ONLY: [&str; 2] = ["description", "cluster"];

#[test]
fn fixtures_decode_to_expected() {
    let idls = common::fixture_idls();
    let fixtures = common::fixtures();
    assert!(
        fixtures.len() >= 13,
        "only {} fixtures found",
        fixtures.len()
    );

    let mut mismatches = Vec::new();
    for (name, tx, expected) in &fixtures {
        let got = serde_json::to_value(decode(tx, &idls)).unwrap();
        for (field, want) in expected.as_object().unwrap() {
            if DOC_ONLY.contains(&field.as_str()) {
                continue;
            }
            if field == "error_message" && want.is_null() {
                continue;
            }
            let have = got.get(field).unwrap_or(&serde_json::Value::Null);
            if have != want {
                mismatches.push(format!("{name}.{field}: expected {want}, got {have}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "\n{}", mismatches.join("\n"));
}

/// The fee is 5000 lamports per signature plus the priority fee, so a correct
/// compute-budget parse must reproduce every fixture's fee exactly.
#[test]
fn derived_priority_fee_matches_the_charged_fee() {
    for (name, tx, _) in common::fixtures() {
        let d = decode(&tx, &failscope_decoder::NoIdls);
        assert_eq!(
            tx.fee,
            5_000 * u64::from(tx.num_signatures) + d.priority_fee,
            "{name}: cu_requested={} priority_fee={}",
            d.cu_requested,
            d.priority_fee
        );
    }
}

#[test]
fn devnet_compute_case_uses_its_explicit_limit() {
    let d = decode(
        &common::fixture("devnet_compute_exhausted"),
        &failscope_decoder::NoIdls,
    );
    assert_eq!(d.cu_requested, 20_000);
}
