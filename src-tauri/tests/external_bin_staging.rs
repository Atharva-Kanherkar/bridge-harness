//! Every declared sidecar is staged, not just the one build.rs used to name.
//!
//! The build script runs before this test compiles, so asserting on the
//! staged files here asserts on exactly the state `tauri_build::build()`
//! validated. On a checkout whose daemon prepare script has never run, the
//! `bridged` placeholder is the one that is missing.

use serde_json::Value;
use std::path::Path;

fn declared_external_bins() -> Vec<String> {
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let body = std::fs::read_to_string(&config).expect("tauri.conf.json is readable");
    let parsed: Value = serde_json::from_str(&body).expect("tauri.conf.json is valid JSON");
    parsed["bundle"]["externalBin"]
        .as_array()
        .expect("bundle.externalBin is an array")
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .expect("every externalBin entry is a string")
                .to_owned()
        })
        .collect()
}

#[test]
fn every_declared_external_bin_is_staged_for_this_target() {
    let target = env!("TAURI_ENV_TARGET_TRIPLE");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let declared = declared_external_bins();
    assert!(
        declared.len() > 1,
        "the fixture is only meaningful while more than one sidecar is declared, found {declared:?}"
    );
    for entry in declared {
        let staged = root.join(format!("{entry}-{target}"));
        assert!(
            staged.exists(),
            "{} was declared in externalBin but nothing staged {}",
            entry,
            staged.display()
        );
    }
}
