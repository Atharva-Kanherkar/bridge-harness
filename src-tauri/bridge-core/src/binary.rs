use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn fallback_directories(home: Option<&Path>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(home) = home {
        directories.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join("bin"),
        ]);
    }
    directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    directories
}

fn hydrated_path_from(existing: Option<&OsStr>, home: Option<&Path>) -> Option<OsString> {
    let mut directories = existing
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    directories.extend(fallback_directories(home));
    let mut seen = HashSet::new();
    directories.retain(|directory| seen.insert(directory.clone()));
    env::join_paths(directories).ok()
}

/// Give provider children the same executable reachability Bridge uses itself.
/// Existing PATH order wins; standard GUI-missing locations are appended once.
pub fn hydrate_command_path(command: &mut Command) {
    if let Some(path) = hydrated_path_from(
        env::var_os("PATH").as_deref(),
        env::var_os("HOME").as_deref().map(Path::new),
    ) {
        command.env("PATH", path);
    }
}

/// Resolve a CLI binary even when Bridge is launched as a macOS .app without a login-shell PATH.
pub fn resolve(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut candidates = fallback_directories(home.as_deref())
        .into_iter()
        .map(|directory| directory.join(name))
        .collect::<Vec<_>>();
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            candidates.push(dir.join(name));
        }
    }
    candidates.into_iter().find(|path| is_executable(path))
}


/// A build identity for an executable: FNV-1a-64 over the file's bytes,
/// hex-encoded.
///
/// Exists because `ServerInfo.version` cannot tell two dev builds apart — the
/// workspace version is a constant — and a desktop app attaching to a daemon
/// built hours earlier silently runs stale code behind a fresh UI. Hand-rolled
/// FNV rather than `DefaultHasher` because the comparison crosses process and
/// toolchain boundaries, and `DefaultHasher` guarantees stability across
/// neither. Not cryptographic on purpose: this distinguishes builds, it does
/// not authenticate them (the socket token does that).
pub fn file_identity(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    Ok(format!("{hash:016x}"))
}

/// [`file_identity`] of the running executable. Callers that outlive rebuilds
/// (the daemon) must call this at startup, while the file at
/// `current_exe()` is still the binary that is actually running.
pub fn self_identity() -> std::io::Result<String> {
    file_identity(&env::current_exe()?)
}

pub fn version(name: &str) -> Option<String> {
    version_at(&resolve(name)?)
}

/// Read `--version` from a specific executable.
///
/// Separate from [`version`] so a caller that already chose which copy to launch
/// reports that copy's version rather than whatever happens to be on PATH.
pub fn version_at(binary: &Path) -> Option<String> {
    let output = Command::new(binary).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_skips_missing_binaries() {
        assert!(resolve("bridge-definitely-missing-binary-xyz").is_none());
    }

    #[test]
    fn hydrated_path_preserves_existing_order_and_appends_fallbacks_once() {
        let existing = env::join_paths(["/custom/bin", "/usr/bin", "/path with spaces"]).unwrap();
        let hydrated = hydrated_path_from(
            Some(&existing),
            Some(Path::new("/Users/test user")),
        )
        .unwrap();
        let directories = env::split_paths(&hydrated).collect::<Vec<_>>();
        assert_eq!(
            &directories[..3],
            &[
                PathBuf::from("/custom/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/path with spaces"),
            ]
        );
        assert_eq!(directories.iter().filter(|path| **path == Path::new("/usr/bin")).count(), 1);
        assert!(directories.contains(&PathBuf::from("/Users/test user/.local/bin")));
        assert!(directories.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn hydrated_path_handles_a_minimal_gui_environment_without_home() {
        let hydrated = hydrated_path_from(Some(OsStr::new("/bin")), None).unwrap();
        let directories = env::split_paths(&hydrated).collect::<Vec<_>>();
        assert_eq!(directories[0], PathBuf::from("/bin"));
        assert!(directories.contains(&PathBuf::from("/usr/local/bin")));
        assert!(directories.contains(&PathBuf::from("/usr/bin")));
    }

    #[cfg(unix)]
    #[test]
    fn a_bare_child_command_resolves_from_the_hydrated_gui_path() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let fixture = bin.join("bridge-path-probe");
        fs::write(&fixture, "#!/bin/sh\nprintf hydrated").unwrap();
        let mut permissions = fs::metadata(&fixture).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&fixture, permissions).unwrap();

        let path = hydrated_path_from(Some(OsStr::new("/usr/bin:/bin")), Some(home.path())).unwrap();
        let output = Command::new("bridge-path-probe")
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"hydrated");
    }

    /// The identity crosses process and toolchain boundaries, so it has to be
    /// a pure function of the bytes — same content same id, one byte one id.
    #[test]
    fn file_identity_is_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::write(&a, b"the same bytes").unwrap();
        fs::write(&b, b"the same bytes").unwrap();
        let first = file_identity(&a).unwrap();
        assert_eq!(first.len(), 16, "a fixed-width hex id");
        assert_eq!(first, file_identity(&a).unwrap(), "stable across reads");
        assert_eq!(first, file_identity(&b).unwrap(), "a function of content, not path");
        fs::write(&b, b"the same bytez").unwrap();
        assert_ne!(first, file_identity(&b).unwrap(), "one byte, one id");
    }

    #[test]
    fn a_missing_binary_identity_is_an_error_not_a_panic() {
        assert!(file_identity(Path::new("/nonexistent/bridged")).is_err());
    }

    #[test]
    fn self_identity_matches_the_file_identity_of_the_test_binary() {
        let own = env::current_exe().unwrap();
        assert_eq!(self_identity().unwrap(), file_identity(&own).unwrap());
    }
}

/// Installed Tauri Linux bundles put resources beside `usr/bin` in
/// `usr/lib/bridge-deck`. The relative path also works inside an AppImage.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn linux_resource_path(executable: &Path, resource: &str) -> Option<PathBuf> {
    Some(executable.parent()?.join("../lib/bridge-deck").join(resource))
}

#[cfg(test)]
mod linux_resource_tests {
    use super::*;

    #[test]
    fn installed_and_appimage_sidecars_resolve_without_the_source_checkout() {
        let root = tempfile::tempdir().unwrap();
        for prefix in ["debian", "appimage"] {
            let usr = root.path().join(prefix).join("usr");
            fs::create_dir_all(usr.join("bin")).unwrap();
            for resource in ["sidecar/claude-agent/index.mjs", "sidecar/terminal-state/index.mjs"] {
                let installed = usr.join("lib/bridge-deck").join(resource);
                fs::create_dir_all(installed.parent().unwrap()).unwrap();
                fs::write(&installed, "bundled runtime").unwrap();
                let resolved = linux_resource_path(&usr.join("bin/bridged"), resource).unwrap();
                assert_eq!(fs::read_to_string(resolved).unwrap(), "bundled runtime");
            }
        }
    }
}
