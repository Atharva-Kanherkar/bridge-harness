//! Small, bounded native diagnostics that survive Finder launches (no terminal).
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Contain both unwind types before returning to a non-unwinding native
/// callback. The Objective-C handler must be inside the Rust panic handler.
/// A fatal runtime abort (such as objc_initWeak) cannot be caught here.
pub fn native_boundary<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(target_os = "macos")]
        {
            objc2::exception::catch(std::panic::AssertUnwindSafe(f)).map_err(|exception| {
                let mut message = format!("Objective-C exception: {exception:?}");
                if let Some(exception) = exception
                    .as_ref()
                    .and_then(|exception| exception.downcast_ref::<objc2_foundation::NSException>())
                {
                    for symbol in exception.callStackSymbols().iter().take(32) {
                        message.push('\n');
                        message.push_str(&symbol.to_string());
                    }
                }
                message
            })
        }
        #[cfg(not(target_os = "macos"))]
        Ok(f())
    }))
    .unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown Rust panic");
        Err(format!("Rust panic: {message}"))
    })
}

pub fn install(directory: Option<PathBuf>) {
    if let Some(directory) = directory {
        if std::fs::create_dir_all(&directory).is_ok() {
            let _ = LOG_PATH.set(directory.join("bridge-diagnostics.log"));
        }
    }
    record(&format!(
        "starting Bridge {} pid={} executable={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    ));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Capture before unwinding reaches a Cocoa callback, where Rust aborts.
        record(&format!(
            "{info}\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        previous(info);
    }));
}

pub fn path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

pub fn record(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "bridge: {message}");
    let Some(path) = path() else {
        return;
    };
    let _guard = LOG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = append(path, message);
}

fn append(path: &Path, message: &str) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() >= MAX_LOG_BYTES) {
        std::fs::rename(path, path.with_extension("previous.log"))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    writeln!(file, "{} {message}", chrono::Utc::now().to_rfc3339())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_boundary_preserves_results_and_contains_rust_panics() {
        assert_eq!(super::native_boundary(|| 42), Ok(42));
        assert_eq!(
            super::native_boundary(|| panic!("startup fixture")),
            Err("Rust panic: startup fixture".into())
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn native_boundary_catches_objective_c_before_rust_unwinding() {
        use objc2_foundation::{NSException, NSString};
        let result = super::native_boundary(|| {
            // NSException is an Objective-C exception object; no userInfo is
            // passed, so there are no dictionary generic requirements.
            let exception = unsafe {
                NSException::exceptionWithName_reason_userInfo(
                    &NSString::from_str("BridgeFixtureException"),
                    Some(&NSString::from_str("native boundary fixture")),
                    None,
                )
            };
            objc2::exception::throw(unsafe { objc2::rc::Retained::cast(exception) });
        });
        assert!(result.unwrap_err().contains("BridgeFixtureException"));
    }

    #[test]
    fn diagnostics_rotate_and_keep_the_previous_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bridge-diagnostics.log");
        super::append(&path, "original failure").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(super::MAX_LOG_BYTES)
            .unwrap();
        super::append(&path, "new startup").unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("new startup"));
        assert!(std::fs::read(path.with_extension("previous.log"))
            .unwrap()
            .windows(b"original failure".len())
            .any(|w| w == b"original failure"));
    }
}
