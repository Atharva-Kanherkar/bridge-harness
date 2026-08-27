//! Stage a placeholder for every declared external binary before Tauri
//! validates them.
//!
//! `tauri_build::build()` checks each `bundle.externalBin` path exists — during
//! `cargo check`, and long before the prepare scripts have built the real
//! sidecars. Each prepare script creates only its own placeholder, and the
//! first one to run invokes cargo, so on a checkout that has never been built
//! the later sidecars are still missing when validation happens.
//!
//! Reading the declared list rather than restating one member of it is what
//! keeps that fixed: a sidecar added to `tauri.conf.json` is staged here
//! without this file changing. Release and dev setup replace these placeholders
//! with the real binaries before bundling.

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
    tauri_build::build()
}
