//! `bridged` entry point: flag parsing, signal handling, and loud failures.
//!
//! ```text
//! bridged --data-dir <path> [--socket <path>] [--health-addr <ip:port>|none]
//!         [--browser-extension <path>]
//! ```

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

const DEFAULT_HEALTH_ADDR: &str = "127.0.0.1:4318";

fn main() -> ExitCode {
    let helper_args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(
        helper_args.first().map(String::as_str),
        Some("--bridge-keychain-read" | "--bridge-keychain-read-interactive")
    ) {
        return keychain_read_helper(&helper_args);
    }
    let config = match parse_flags(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("bridged: {message}");
            eprintln!(
                "usage: bridged --data-dir <path> [--socket <path>] \
                 [--health-addr <ip:port>|none] [--browser-extension <path>]"
            );
            return ExitCode::from(2);
        }
    };

    let (daemon, listener) = match bridged::Daemon::start(config) {
        Ok(started) => started,
        Err(error) => {
            // Startup failures are always visible: print and exit non-zero.
            eprintln!("bridged: {error}");
            return ExitCode::FAILURE;
        }
    };

    // SIGINT/SIGTERM request a graceful shutdown; the accept loop polls the
    // flag and exits, after which the daemon drains in-flight connections,
    // stops adapters, and cleans up.
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        let state = daemon.state.clone();
        unsafe {
            let _ = signal_hook::low_level::register(signal, move || {
                state.shutting_down.store(true, Ordering::SeqCst);
            });
        }
    }

    eprintln!(
        "bridged: serving {} (data dir {})",
        daemon.socket_path.display(),
        daemon
            .core
            .database_path
            .parent()
            .unwrap_or(&daemon.socket_path)
            .display()
    );
    let served = bridged::serve(&daemon, listener);
    daemon.shutdown(bridged::DEFAULT_DRAIN_TIMEOUT);
    match served {
        Ok(()) => {
            eprintln!("{}", bridged::CLEAN_SHUTDOWN_MARKER);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bridged: accept loop failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn keychain_read_helper(args: &[String]) -> ExitCode {
    let Some(service) = args.get(1) else {
        return ExitCode::from(2);
    };
    if args.len() > 3
        || !["Claude Code-credentials", "dev.bridge.deck.provider-usage"]
            .contains(&service.as_str())
    {
        return ExitCode::from(2);
    }
    let interactive = args[0] == "--bridge-keychain-read-interactive";
    match bridge_core::provider_usage::credentials::keychain_helper_read(
        service,
        args.get(2).map(String::as_str),
        interactive,
    ) {
        Ok(bytes) if bytes.len() <= 65_536 => {
            if std::io::stdout().write_all(&bytes).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => ExitCode::FAILURE,
    }
}

fn parse_flags(args: impl Iterator<Item = String>) -> Result<bridged::DaemonConfig, String> {
    let mut data_dir: Option<PathBuf> = None;
    let mut socket_path: Option<PathBuf> = None;
    let mut health: Option<String> = None;
    let mut browser_extension: Option<PathBuf> = None;
    let mut args = args.peekable();
    while let Some(flag) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(value("--data-dir")?)),
            "--socket" => socket_path = Some(PathBuf::from(value("--socket")?)),
            "--health-addr" => health = Some(value("--health-addr")?),
            "--browser-extension" => {
                browser_extension = Some(PathBuf::from(value("--browser-extension")?))
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let data_dir = data_dir
        .or_else(|| std::env::var_os("BRIDGE_DATA_DIR").map(PathBuf::from))
        .or_else(default_data_dir)
        .ok_or("--data-dir is required (or set BRIDGE_DATA_DIR)")?;
    let health_addr: Option<SocketAddr> = match health.as_deref() {
        None => Some(
            DEFAULT_HEALTH_ADDR
                .parse()
                .expect("default health addr parses"),
        ),
        Some("none") => None,
        Some(addr) => Some(
            addr.parse()
                .map_err(|_| format!("--health-addr must be ip:port or none, got {addr}"))?,
        ),
    };
    let browser_extension_path = browser_extension.unwrap_or_else(default_browser_extension_path);
    Ok(bridged::DaemonConfig {
        data_dir,
        socket_path,
        health_addr,
        browser_extension_path,
        handshake_timeout: bridged::DEFAULT_HANDSHAKE_TIMEOUT,
    })
}

/// The bundled browser extension, resolved at runtime. `bridged` ships as a
/// Tauri external binary in `Contents/MacOS/`, next to the app binary and one
/// step from `Contents/Resources/` where the extension is bundled; a source
/// checkout falls back to the in-tree copy. The compile-time path is a last
/// resort for `cargo run` from an uninstalled build.
fn default_browser_extension_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("BRIDGE_BROWSER_EXTENSION") {
        return PathBuf::from(explicit);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            for candidate in [
                bin_dir.join("../Resources/browser-extension"), // macOS app bundle
                bin_dir.join("browser-extension"),              // flat layouts
            ] {
                if candidate.is_dir() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../browser-extension")
}

/// The desktop app's data directory, so `bridged` with no flags owns the same
/// data the app uses (never concurrently — the lease enforces that).
fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/dev.bridge.deck"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|base| base.join("dev.bridge.deck"))
    }
}
