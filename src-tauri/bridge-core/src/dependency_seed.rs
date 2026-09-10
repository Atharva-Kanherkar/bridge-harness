//! Seed dependencies only when the checkout describes the same installation.
//! Never symlink or hardlink node_modules: worker writes must stay isolated.
use std::{path::Path, process::Command, time::Duration};

pub fn seed(source: &Path, destination: &Path) -> &'static str {
    let modules = source.join("node_modules");
    if !modules.symlink_metadata().is_ok_and(|meta| meta.is_dir()) || destination.join("node_modules").exists() {
        return "not seeded: no independent source installation or destination already populated";
    }
    let Ok(files) = crate::git::command_output_deadline(
        crate::git::git_command(destination).args(["ls-files", "-z"]), Duration::from_secs(10),
    ) else { return "not seeded: could not verify dependency manifests"; };
    if !files.status.success() { return "not seeded: could not verify dependency manifests"; }
    let mut has_lock = false;
    for name in files.stdout.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
        let Ok(name) = std::str::from_utf8(name) else { return "not seeded: unsupported manifest path"; };
        let path = Path::new(name);
        let file = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if !matches!(file, "package.json" | "bun.lock" | "bun.lockb" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | ".npmrc" | "bunfig.toml") { continue; }
        has_lock |= matches!(file, "bun.lock" | "bun.lockb" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock");
        if !same_file(&source.join(path), &destination.join(path)) { return "not seeded: dependency manifests differ; run the normal install"; }
    }
    if !has_lock { return "not seeded: no lockfile; run the normal install"; }
    let staging = destination.join(format!(".bridge-dependency-seed-{}", uuid::Uuid::new_v4()));
    let mut command = Command::new("cp");
    #[cfg(target_os = "macos")]
    command.args(["-cR"]);
    #[cfg(not(target_os = "macos"))]
    command.args(["-R", "--reflink=always"]);
    let cloned = crate::git::command_output_deadline(command.arg(&modules).arg(&staging), Duration::from_secs(30))
        .is_ok_and(|output| output.status.success());
    if cloned && std::fs::rename(&staging, destination.join("node_modules")).is_ok() {
        "seeded: independent copy-on-write dependencies; run the normal install after dependency changes"
    } else {
        // Only the staging directory minted above, never a user's installation.
        let _ = std::fs::remove_dir_all(staging);
        "not seeded: copy-on-write unavailable; run the normal install"
    }
}

fn same_file(source: &Path, destination: &Path) -> bool {
    match (std::fs::read(source), std::fs::read(destination)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_lockfile_never_exposes_source_dependencies() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(source.join("node_modules")).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        for args in [vec!["init"], vec!["add", "bun.lock", "package.json"]] {
            if args[0] == "add" {
                std::fs::write(dest.join("bun.lock"), "new").unwrap();
                std::fs::write(dest.join("package.json"), "{}").unwrap();
            }
            assert!(Command::new("git").current_dir(&dest).args(args).status().unwrap().success());
        }
        std::fs::write(source.join("bun.lock"), "old").unwrap();
        std::fs::write(source.join("package.json"), "{}").unwrap();
        assert!(seed(&source, &dest).contains("differ"));
        assert!(!dest.join("node_modules").exists());
    }

    #[test]
    fn matching_manifests_clone_independently_or_leave_install_to_the_worker() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let dest = root.path().join("dest");
        std::fs::create_dir_all(source.join("node_modules")).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        for path in [&source, &dest] {
            std::fs::write(path.join("bun.lock"), "same").unwrap();
            std::fs::write(path.join("package.json"), "{}").unwrap();
        }
        std::fs::write(source.join("node_modules/dependency"), "original").unwrap();
        assert!(Command::new("git").current_dir(&dest).arg("init").status().unwrap().success());
        assert!(Command::new("git").current_dir(&dest).args(["add", "bun.lock", "package.json"]).status().unwrap().success());
        let result = seed(&source, &dest);
        eprintln!("dependency seed observation: {result}");
        if result.starts_with("seeded:") {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_ne!(std::fs::metadata(source.join("node_modules/dependency")).unwrap().ino(), std::fs::metadata(dest.join("node_modules/dependency")).unwrap().ino(), "clones must not be hardlinks");
            }
            assert_eq!(std::fs::read_to_string(dest.join("node_modules/dependency")).unwrap(), "original");
            std::fs::write(dest.join("node_modules/dependency"), "worker changed").unwrap();
            assert_eq!(std::fs::read_to_string(source.join("node_modules/dependency")).unwrap(), "original");
        } else {
            assert!(result.contains("copy-on-write unavailable"), "{result}");
            assert!(!dest.join("node_modules").exists());
        }
    }
}
