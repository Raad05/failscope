//! Decodes every fixture and prints a markdown table plus coverage by source.
//!
//!     cargo run -p failscope-decoder --example decode_fixtures

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use failscope_decoder::{decode, from_rpc_json, IdlErrorTable};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");

    let mut idls = IdlErrorTable::new();
    for entry in fs::read_dir(root.join("idls"))? {
        let path = entry?.path();
        let program_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        idls.add_idl(program_id, &serde_json::from_str(&fs::read_to_string(&path)?)?)?;
    }

    let mut dirs: Vec<_> = fs::read_dir(root.join("txs"))?.collect::<Result<_, _>>()?;
    dirs.sort_by_key(|d| d.path());

    let mut coverage: BTreeMap<String, usize> = BTreeMap::new();
    println!("| Fixture | Failing program (depth) | Code | Decoded as | Source | Confidence |");
    println!("|---|---|---|---|---|---|");
    for dir in &dirs {
        let tx = from_rpc_json(&serde_json::from_str(&fs::read_to_string(
            dir.path().join("tx.json"),
        )?)?)?;
        let d = decode(&tx, &idls);
        let program = d.failing_program_id.as_deref().unwrap_or("-");
        let source = serde_json::to_value(d.decode_source)?;
        let confidence = serde_json::to_value(d.attribution_confidence)?;
        println!(
            "| `{}` | `{}…` ({}) | {} | {} | {} | {} |",
            dir.file_name().to_string_lossy(),
            program.get(..6).unwrap_or(program),
            d.cpi_depth.map_or("?".to_string(), |n| n.to_string()),
            d.error_code.map_or("-".to_string(), |c| c.to_string()),
            d.error_name.as_deref().unwrap_or("_unknown_"),
            source.as_str().unwrap_or_default(),
            confidence.as_str().unwrap_or_default(),
        );
        *coverage
            .entry(source.as_str().unwrap_or_default().to_string())
            .or_default() += 1;
    }
    println!("\nCoverage by decode_source:");
    for (source, n) in coverage {
        println!("- {source}: {n}");
    }
    Ok(())
}
