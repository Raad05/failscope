//! Shared fixture loading for the decoder's integration tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use failscope_decoder::{from_rpc_json, IdlErrorTable, TxInput};

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every IDL in fixtures/idls, keyed by file stem (the program id).
pub fn fixture_idls() -> IdlErrorTable {
    let mut table = IdlErrorTable::new();
    for entry in fs::read_dir(fixtures_dir().join("idls")).unwrap() {
        let path = entry.unwrap().path();
        let program_id = path.file_stem().unwrap().to_str().unwrap().to_string();
        table.add_idl(&program_id, &read_json(&path)).unwrap();
    }
    table
}

/// (name, tx input, expected.json) for every fixture, sorted by name.
pub fn fixtures() -> Vec<(String, TxInput, serde_json::Value)> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(fixtures_dir().join("txs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| {
            let name = dir.file_name().unwrap().to_str().unwrap().to_string();
            let tx = from_rpc_json(&read_json(&dir.join("tx.json")))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, tx, read_json(&dir.join("expected.json")))
        })
        .collect()
}

pub fn fixture(name: &str) -> TxInput {
    fixtures()
        .into_iter()
        .find(|(n, _, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}
