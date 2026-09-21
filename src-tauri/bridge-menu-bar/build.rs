use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=swift");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("Unsupported macOS architecture: {other}"),
    };
    let target = format!("{arch}-apple-macosx12.0");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut sources: Vec<_> = std::fs::read_dir("swift")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "swift"))
        .collect();
    sources.sort();
    let result = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-swift-version",
            "5",
            "-module-name",
            "BridgeMenuBar",
            "-target",
            &target,
        ])
        .arg(if env::var("PROFILE").as_deref() == Ok("release") {
            "-O"
        } else {
            "-Onone"
        })
        .arg("-module-cache-path")
        .arg(out.join("swift-module-cache"))
        .args(&sources)
        .arg("-o")
        .arg(out.join("libBridgeMenuBar.a"))
        .output()
        .expect("Xcode's Swift compiler is required for the macOS menu bar");
    assert!(
        result.status.success(),
        "Menu Bar Swift compilation failed:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let info = Command::new("xcrun")
        .args(["swiftc", "-print-target-info", "-target", &target])
        .output()
        .unwrap();
    assert!(info.status.success(), "Could not resolve the Swift runtime");
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    for path in info["paths"]["runtimeLibraryPaths"].as_array().unwrap() {
        println!("cargo:rustc-link-search=native={}", path.as_str().unwrap());
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=BridgeMenuBar");
    for framework in ["AppKit", "SwiftUI", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
