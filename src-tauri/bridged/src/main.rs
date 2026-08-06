//! `bridged` entry point: flag parsing, signal handling, and loud failures.
//!
//! ```text
//! bridged --data-dir <path> [--socket <path>] [--health-addr <ip:port>|none]
//!         [--browser-extension <path>]
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

const DEFAULT_HEALTH_ADDR: &str = "127.0.0.1:4318";

fn main() -> ExitCode {
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
    // flag and exits, after which the daemon stops adapters and cleans up.
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
        daemon.core.database_path.parent().unwrap_or(&daemon.socket_path).display()
    );
    let served = bridged::serve(&daemon, listener);
    daemon.shutdown();
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bridged: accept loop failed: {error}");
            ExitCode::FAILURE
        }
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
            args.next().ok_or_else(|| format!("{flag} requires a value"))
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
        None => Some(DEFAULT_HEALTH_ADDR.parse().expect("default health addr parses")),
        Some("none") => None,
        Some(addr) => Some(
            addr.parse()
                .map_err(|_| format!("--health-addr must be ip:port or none, got {addr}"))?,
        ),
    };
    let browser_extension_path = browser_extension.unwrap_or_else(|| {
        // The bundled extension sits next to the desktop app's resources in
        // production; in a repo checkout, use the in-tree copy.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../browser-extension")
    });
    Ok(bridged::DaemonConfig {
        data_dir,
        socket_path,
        health_addr,
        browser_extension_path,
    })
}

/// The desktop app's data directory, so `bridged` with no flags owns the same
/// data the app uses (never concurrently — the lease enforces that).
fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home).join("Library/Application Support/dev.bridge.deck")
        })
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
