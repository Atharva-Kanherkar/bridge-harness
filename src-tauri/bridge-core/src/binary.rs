use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Resolve a CLI binary even when Bridge is launched as a macOS .app without a login-shell PATH.
pub fn resolve(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = &home {
        candidates.extend([
            home.join(".local/bin").join(name),
            home.join(".cargo/bin").join(name),
            home.join("bin").join(name),
        ]);
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
        PathBuf::from("/usr/bin").join(name),
    ]);
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
