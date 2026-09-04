//! Release-bundle contract: version, DMG target, and the Claude sidecar path
//! the adapter already looks for inside a macOS .app.

use serde_json::Value;
use std::path::Path;

fn tauri_conf() -> Value {
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let body = std::fs::read_to_string(&config).expect("tauri.conf.json is readable");
    serde_json::from_str(&body).expect("tauri.conf.json is valid JSON")
}

#[test]
fn release_version_is_0_5_0() {
    let parsed = tauri_conf();
    assert_eq!(parsed["version"], "0.5.0");
}

#[test]
fn bundle_targets_include_app_and_dmg() {
    let parsed = tauri_conf();
    let targets = parsed["bundle"]["targets"]
        .as_array()
        .expect("bundle.targets is an array");
    let as_str: Vec<&str> = targets.iter().filter_map(Value::as_str).collect();
    assert!(as_str.contains(&"app"), "targets={as_str:?}");
    assert!(as_str.contains(&"dmg"), "targets={as_str:?}");
}

#[test]
fn claude_sidecar_is_bundled_under_resources() {
    let parsed = tauri_conf();
    let resources = parsed["bundle"]["resources"]
        .as_object()
        .expect("bundle.resources is an object");
    let dest = resources
        .values()
        .filter_map(Value::as_str)
        .find(|path| *path == "sidecar/claude-agent/");
    assert_eq!(
        dest,
        Some("sidecar/claude-agent/"),
        "adapter sidecar_entry looks at Contents/Resources/sidecar/claude-agent/index.mjs; resources={resources:?}"
    );
    assert_eq!(
        resources
            .get("resources/sidecar/claude-agent/")
            .and_then(Value::as_str),
        Some("sidecar/claude-agent/"),
        "prepare:claude-sidecar stages a real npm tree here because bun workspace hoists are symlinks"
    );
}
