fn main() {
    // Tauri validates sidecar paths even during `cargo check`, before the
    // preparation script has built the real native host. Keep that generated
    // path present without committing a platform binary; release/dev setup
    // replaces this placeholder before bundling.
    if let Ok(target) = std::env::var("TARGET") {
        let path =
            std::path::PathBuf::from("binaries").join(format!("bridge-browser-host-{target}"));
        if !path.exists() {
            let _ = std::fs::create_dir_all("binaries");
            let _ = std::fs::write(&path, b"#!/bin/sh\nexit 1\n");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    tauri_build::build()
}
