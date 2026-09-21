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
fn release_versions_match_across_packages() {
    let parsed = tauri_conf();
    assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    let npm: Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
    assert_eq!(parsed["version"], npm["version"]);
}

#[test]
fn window_creation_errors_are_handled_by_the_setup_hook() {
    for window in tauri_conf()["app"]["windows"].as_array().unwrap() {
        assert_eq!(
            window["create"], false,
            "automatic window creation fails before Bridge can report a startup error"
        );
    }
}

#[test]
fn main_window_delivers_html_drag_and_drop_to_the_frontend() {
    let config: tauri::utils::config::Config = serde_json::from_value(tauri_conf()).unwrap();
    let main = config.app.windows.first().expect("main window config");
    assert!(
        !main.drag_drop_enabled,
        "Tauri's native handler consumes drag/drop before WKWebView can deliver it to Mission Control and Agent Fleet"
    );
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
fn macos_bundle_ships_webkit_jit_entitlements() {
    let parsed = tauri_conf();
    assert_eq!(
        parsed["bundle"]["macOS"]["hardenedRuntime"], true,
        "Developer ID notarization requires Hardened Runtime"
    );
    assert_eq!(
        parsed["bundle"]["macOS"]["entitlements"], "entitlements.plist",
        "without an entitlements file, codesign embeds an empty blob and WKWebView aborts"
    );

    let plist = Path::new(env!("CARGO_MANIFEST_DIR")).join("entitlements.plist");
    let body = std::fs::read_to_string(&plist).expect("entitlements.plist is readable");
    for key in [
        "com.apple.security.cs.allow-jit",
        "com.apple.security.cs.allow-unsigned-executable-memory",
        "com.apple.security.cs.disable-library-validation",
    ] {
        assert!(
            body.contains(key),
            "entitlements.plist must grant {key} so JavaScriptCore can run under Hardened Runtime"
        );
    }
}

#[test]
fn macos_bundle_declares_microphone_access() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Info.plist");
    let manifest_body = std::fs::read_to_string(&manifest).expect("Info.plist is readable");
    assert!(
        manifest_body.contains("<key>NSMicrophoneUsageDescription</key>")
            && manifest_body.contains("transcribe text into the composer"),
        "the packaged app must explain composer dictation before macOS prompts for microphone access"
    );

    let entitlements = Path::new(env!("CARGO_MANIFEST_DIR")).join("entitlements.plist");
    let entitlement_body =
        std::fs::read_to_string(&entitlements).expect("entitlements.plist is readable");
    assert!(
        entitlement_body
            .contains("<key>com.apple.security.device.audio-input</key>\n\t<true/>"),
        "the signed app must retain the audio-input entitlement"
    );
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
