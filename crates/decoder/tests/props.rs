//! Property tests: the log parser and the decoder never panic, whatever the logs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use failscope_decoder::{analyze_logs, decode, parse_anchor_error, Confidence, NoIdls};
use proptest::prelude::*;

const ID: &str = "6MzbWCZgmdSrBwcUVypa4aGNKdck6r49jV7Y48AhPNey";

/// Lines shaped like real log lines, with random holes and junk.
fn log_line() -> impl Strategy<Value = String> {
    prop_oneof![
        (0u32..10).prop_map(|d| format!("Program {ID} invoke [{d}]")),
        Just(format!("Program {ID} success")),
        ".{0,40}".prop_map(|r| format!("Program {ID} failed: {r}")),
        ".{0,80}".prop_map(|r| format!("Program log: AnchorError {r}")),
        (".{0,20}", any::<i64>(), ".{0,20}").prop_map(|(n, num, m)| format!(
            "Program log: AnchorError occurred. Error Code: {n}. Error Number: {num}. Error Message: {m}."
        )),
        Just("Log truncated".to_string()),
        Just(format!("Program {ID} invoke [")),
        Just("Program  failed: ".to_string()),
        ".{0,60}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn analyze_logs_never_panics(lines in prop::collection::vec(log_line(), 0..40)) {
        let _ = analyze_logs(&lines);
    }

    #[test]
    fn anchor_parser_never_panics(s in ".{0,200}") {
        let _ = parse_anchor_error(&s);
    }

    /// Cutting any real fixture's logs at any point must not panic. If the
    /// `failed:` line is cut off and CPIs ran, attribution must be low.
    #[test]
    fn any_prefix_of_real_logs_is_handled(fixture_ix in 0usize..12, cut_frac in 0.0f64..1.0) {
        let fixtures = common::fixtures();
        let (name, mut tx, _) = fixtures[fixture_ix % fixtures.len()].clone();
        let logs = tx.logs.clone().unwrap();
        let cut = (logs.len() as f64 * cut_frac) as usize;
        let kept: Vec<String> = logs.iter().take(cut).cloned().collect();
        let has_failed_line = kept.iter().any(|l| l.contains(" failed: "));
        let ix = match &tx.err {
            failscope_decoder::TransactionError::InstructionError(i, _) => *i,
            _ => unreachable!(),
        };
        let cpis = tx.inner_for(ix).is_some_and(|g| !g.instructions.is_empty());
        tx.logs = Some(kept);
        let d = decode(&tx, &NoIdls);
        if !has_failed_line && cpis {
            prop_assert_eq!(d.attribution_confidence, Confidence::Low, "{}", name);
        }
    }
}
