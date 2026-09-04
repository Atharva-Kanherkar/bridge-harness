//! Stage a placeholder for every declared external binary and bundle
//! resource before Tauri validates them.
//!
//! `tauri_build::build()` checks each `bundle.externalBin` and `bundle.resources`
//! path exists — during `cargo check`, and long before the prepare scripts
//! have built the real sidecars. Each prepare script creates only its own
//! placeholder, and the first one to run invokes cargo, so on a checkout
//! that has never been built the later sidecars are still missing when
//! validation happens. `resources/sidecar/claude-agent/` is gitignored and
//! only materialized by `scripts/prepare-claude-sidecar.sh` for a real
//! release bundle, so it is never present on a fresh checkout either.
//!
//! Reading the declared lists rather than restating their members is what
//! keeps that fixed: a sidecar or resource added to `tauri.conf.json` is
//! staged here without this file changing. Release and dev setup replace
//! these placeholders with the real content before bundling.

use std::path::{Path, PathBuf};

fn declared_external_bins(config: &Path) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    parsed
        .get("bundle")
        .and_then(|bundle| bundle.get("externalBin"))
        .and_then(|entries| entries.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn declared_resource_sources(config: &Path) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    parsed
        .get("bundle")
        .and_then(|bundle| bundle.get("resources"))
        .and_then(|entries| entries.as_object())
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

fn stage_placeholder_dir(path: &Path) {
    if path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(path);
}

fn stage_placeholder(path: &PathBuf) {
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, b"#!/bin/sh\nexit 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
}

fn main() {
    let config = Path::new("tauri.conf.json");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    if let Ok(target) = std::env::var("TARGET") {
        for entry in declared_external_bins(config) {
            stage_placeholder(&PathBuf::from(format!("{entry}-{target}")));
        }
    }
    for source in declared_resource_sources(config) {
        stage_placeholder_dir(&PathBuf::from(source));
    }
    tauri_build::build()
}
