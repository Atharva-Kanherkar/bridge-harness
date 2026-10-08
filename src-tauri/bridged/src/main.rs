//! `bridged` entry point: flag parsing, signal handling, and loud failures.
//!
//! ```text
//! bridged --data-dir <path> [--socket <path>] [--health-addr <ip:port>|none]
//!         [--browser-extension <path>]
//!         [--listen <ip:port> [--allowed-origin <origin>]... [--allow-remote-bind]]
//!         [--credential-policy user-managed|api-key-only|enterprise-managed]
//!         [--topology embedded|local-daemon|remote-runner]
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
    // The `bridge` MCP server a chat harness spawns for the `visualize` tool:
    // stdio JSON-RPC until the harness closes stdin. Never touches the data
    // directory, so it runs alongside the daemon that owns it.
    if helper_args.first().map(String::as_str) == Some(bridge_core::mcp_apps::HELPER_FLAG) {
        return match bridge_core::mcp_apps::serve_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("bridged: visualize server stopped: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let (config, remote_config, deployment) = match parse_flags(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("bridged: {message}");
            eprintln!(
                "usage: bridged --data-dir <path> [--socket <path>] \
                 [--health-addr <ip:port>|none] [--browser-extension <path>] \
                 [--listen <ip:port> [--allowed-origin <origin>]... [--allow-remote-bind]] \
                 [--credential-policy user-managed|api-key-only|enterprise-managed] \
                 [--topology embedded|local-daemon|remote-runner]"
            );
            return ExitCode::from(2);
        }
    };

    // Pinned before the runtime boots so no harness can start under another
    // policy, and reported on every `health/health`.
    bridge_core::credential_policy::configure(deployment);
    let (daemon, listener) = match bridged::Daemon::start(config) {
        Ok(started) => started,
        Err(error) => {
            // Startup failures are always visible: print and exit non-zero.
            eprintln!("bridged: {error}");
            return ExitCode::FAILURE;
        }
    };

    let remote = match remote_config.as_ref().map(|config| daemon.bind_remote(config)) {
        Some(Ok(listener)) => Some(listener),
        Some(Err(error)) => {
            eprintln!("bridged: {error}");
            return ExitCode::FAILURE;
        }
        None => None,
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
    if let Some(remote) = &remote {
        eprintln!(
            "bridged: remote WebSocket listener on {} (plaintext; token required)",
            remote.local_addr().map(|addr| addr.to_string()).unwrap_or_default()
        );
    }
    let served = bridged::serve_with_remote(&daemon, listener, remote);
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

fn parse_flags(
    args: impl Iterator<Item = String>,
) -> Result<
    (
        bridged::DaemonConfig,
        Option<bridged::RemoteConfig>,
        bridge_core::credential_policy::DeploymentInfo,
    ),
    String,
> {
    let mut data_dir: Option<PathBuf> = None;
    let mut socket_path: Option<PathBuf> = None;
    let mut health: Option<String> = None;
    let mut browser_extension: Option<PathBuf> = None;
    let mut listen: Option<String> = None;
    let mut allowed_origins: Vec<String> = Vec::new();
    let mut allow_remote_bind = false;
    let mut credential_policy: Option<String> = None;
    let mut topology: Option<String> = None;
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
            "--listen" => listen = Some(value("--listen")?),
            "--allowed-origin" => allowed_origins.push(value("--allowed-origin")?),
            "--allow-remote-bind" => allow_remote_bind = true,
            "--credential-policy" => credential_policy = Some(value("--credential-policy")?),
            "--topology" => topology = Some(value("--topology")?),
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
    let remote = match listen {
        Some(addr) => {
            let addr: SocketAddr = addr
                .parse()
                .map_err(|_| format!("--listen must be ip:port, got {addr}"))?;
            Some(bridged::RemoteConfig::new(addr, allowed_origins, allow_remote_bind)?)
        }
        None if !allowed_origins.is_empty() || allow_remote_bind => {
            return Err("--allowed-origin and --allow-remote-bind need --listen".into())
        }
        None => None,
    };
    let deployment = resolve_deployment(credential_policy, topology)?;
    Ok((
        bridged::DaemonConfig {
            data_dir,
            socket_path,
            health_addr,
            browser_extension_path,
            handshake_timeout: bridged::DEFAULT_HANDSHAKE_TIMEOUT,
        },
        remote,
        deployment,
    ))
}

/// Flags win over the environment, which wins over the defaults. A daemon is a
/// `local-daemon` unless told otherwise; a flag that does not parse is an
/// error, never a silent fallback to a looser policy.
fn resolve_deployment(
    policy_flag: Option<String>,
    topology_flag: Option<String>,
) -> Result<bridge_core::credential_policy::DeploymentInfo, String> {
    use bridge_core::credential_policy::{self as policy, CredentialPolicy, ExecutionTopology};
    let from_env = policy::active();
    let credential_policy = match policy_flag {
        Some(value) => CredentialPolicy::parse(&value).ok_or_else(|| {
            format!(
                "--credential-policy must be user-managed, api-key-only or enterprise-managed, got {value}"
            )
        })?,
        None => from_env.credential_policy,
    };
    let topology = match topology_flag {
        Some(value) => ExecutionTopology::parse(&value).ok_or_else(|| {
            format!("--topology must be embedded, local-daemon or remote-runner, got {value}")
        })?,
        None if std::env::var_os(policy::TOPOLOGY_ENV).is_some() => from_env.topology,
        None => ExecutionTopology::LocalDaemon,
    };
    Ok(bridge_core::credential_policy::DeploymentInfo { topology, credential_policy })
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
