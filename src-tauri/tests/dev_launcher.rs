//! The public dev command must finish native preparation before starting
//! Tauri's bounded wait for Vite, and must preserve CLI arguments and failures.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(args: &[&str], fail_prepare: bool) -> (std::process::Output, String) {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("project with spaces");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    fs::create_dir_all(root.join("test-bin")).unwrap();
    fs::write(
        root.join("scripts/tauri.sh"),
        include_str!("../../scripts/tauri.sh"),
    )
    .unwrap();
    executable(
        &root.join("test-bin/bun"),
        r#"#!/bin/sh
printf 'prepare:%s\n' "$2" >> "$BRIDGE_TEST_TRACE"
if [ "$BRIDGE_TEST_PREP_FAIL" = 1 ]; then exit 23; fi
"#,
    );
    executable(
        &root.join("node_modules/.bin/tauri"),
        r#"#!/bin/sh
printf 'tauri\n' >> "$BRIDGE_TEST_TRACE"
for arg in "$@"; do printf 'arg:%s\n' "$arg" >> "$BRIDGE_TEST_TRACE"; done
"#,
    );
    let trace = root.join("trace");
    let path = std::env::join_paths(std::iter::once(root.join("test-bin")).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let output = Command::new("/bin/sh")
        .arg(root.join("scripts/tauri.sh"))
        .args(args)
        .current_dir(fixture.path())
        .env("PATH", path)
        .env("BRIDGE_TEST_TRACE", &trace)
        .env(
            "BRIDGE_TEST_PREP_FAIL",
            if fail_prepare { "1" } else { "0" },
        )
        .output()
        .unwrap();
    (output, fs::read_to_string(trace).unwrap())
}

#[test]
fn native_helpers_are_ready_before_tauri_starts_and_arguments_stay_intact() {
    let (output, trace) = run(
        &["dev", "--no-watch", "--config", "path with spaces.json"],
        false,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(trace, "prepare:prepare:browser-host:dev\nprepare:prepare:daemon:dev\ntauri\narg:dev\narg:--no-watch\narg:--config\narg:path with spaces.json\n");
}

#[test]
fn build_and_other_cli_commands_do_not_prepare_debug_helpers() {
    for args in [vec!["build", "--debug"], vec!["--version"]] {
        let (output, trace) = run(&args, false);
        assert!(output.status.success(), "{output:?}");
        assert!(trace.starts_with("tauri\n"), "{trace}");
        assert!(!trace.contains("prepare:"), "{trace}");
    }
}

#[test]
fn preparation_failure_stops_before_tauri_and_preserves_the_exit_code() {
    let (output, trace) = run(&["dev"], true);
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(trace, "prepare:prepare:browser-host:dev\n");
}
