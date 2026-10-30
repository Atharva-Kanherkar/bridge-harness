//! More than one sign-in for one provider: a second Claude Code or Codex
//! subscription beside the user's own, each chat bound to one of them.
//!
//! Ported from T3 Code's provider instances (https://github.com/pingdotgg/t3code
//! at 97e35bcf, MIT; see THIRD_PARTY_NOTICES.md):
//! `packages/provider-core/src/server/{pathExpansion,instanceEnvironment}.ts`,
//! `apps/server/src/provider/Drivers/ClaudeHome.ts` and `CodexHomeLayout.ts`.
//! An account is T3's instance reduced to what Bridge needs: a name and the
//! provider home it signs in from. Claude reads `CLAUDE_CONFIG_DIR` (never
//! `HOME`, which would hide the Keychain login). Codex reads `CODEX_HOME`,
//! either a separate home or a shadow home that links the shared sessions and
//! configuration and keeps only `auth.json` private, so two accounts can
//! continue one thread.
//!
//! Accounts live in the harness's `advanced.accounts`. No account means the
//! user's own login, exactly as before; the harness id is reserved for it, as
//! T3 reserves the driver kind for its default instance.
//!
//! Which agents take accounts, and how each keeps an account's login apart,
//! is one table: [`DRIVERS`]. T3 lets every driver run several instances; an
//! agent joins here with one entry and one [`apply_account`] call where its
//! adapter builds the process.

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use zeroize::Zeroizing;

const MAX_ID_CHARS: usize = 64;
const MAX_LABEL_CHARS: usize = 64;

/// Directories Codex shares between a home and its shadow, created up front
/// so the first shadow launch links them instead of writing private copies.
const KNOWN_SHARED_DIRECTORIES: [&str; 11] = [
    "sessions",
    "archived_sessions",
    "sqlite",
    "shell_snapshots",
    "worktrees",
    "skills",
    "plugins",
    "cache",
    "logs",
    "mcp-oauth-locks",
    "thread-writer-locks",
];
// auth.lock guards the shadow home's private auth.json.
const PRIVATE_ENTRY_NAMES: [&str; 3] = ["auth.json", "auth.lock", "models_cache.json"];
// Finder keeps metadata per directory, shadow homes included.
const SHADOW_LOCAL_ENTRY_NAMES: [&str; 4] = ["log", "memories", "tmp", ".DS_Store"];
// A shadow home may create these runtime locks before it is linked; they guard
// shared state, so a local copy is replaced by the shared one.
const REPLACEABLE_SHARED_RUNTIME_ENTRIES: [&str; 3] =
    ["mcp-oauth-locks", "thread-writer-locks", ".sqlite-maintenance.lock"];

/// One extra sign-in, as stored in `advanced.accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAccount {
    pub id: String,
    pub label: String,
    /// Claude: the `CLAUDE_CONFIG_DIR`. Codex: the shared `CODEX_HOME`; empty
    /// means the inherited one.
    #[serde(default)]
    pub home_path: String,
    /// Codex only: the account's own home, linked to `home_path`.
    #[serde(default)]
    pub shadow_home_path: String,
}

/// How an agent keeps an account's login apart from the user's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountLogin {
    /// The CLI reads its login from the folder named by `variable`. The user's
    /// own login lives in the inherited value, else `default` under HOME.
    Folder {
        variable: &'static str,
        default: &'static str,
        layout: FolderLayout,
    },
    /// The CLI takes an API key in `variable`. Used where the CLI keeps its own
    /// login somewhere a folder cannot separate; the key stays in the macOS
    /// Keychain ([`crate::account_secrets`]).
    ApiKey { variable: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLayout {
    /// The account's folder is the whole home.
    Direct,
    /// Codex: a separate home, or a shadow home that links a shared one and
    /// keeps its own login.
    CodexShadow,
    /// A data home of which only `private` belongs to the agent; every other
    /// entry links to the user's own, so commands the agent runs still find
    /// their usual data.
    DataOverlay { private: &'static str },
}

/// One agent's account support: T3 Code's driver metadata, reduced to what a
/// Bridge launch needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountDriver {
    pub harness: &'static str,
    pub label: &'static str,
    pub login: AccountLogin,
    /// Inherited variables that would outrank or redirect the account's own
    /// login. An account launch removes them.
    pub overrides: &'static [&'static str],
}

/// Every agent that takes accounts. Adding one is an entry here plus an
/// [`apply_account`] call in its adapter.
pub const DRIVERS: [AccountDriver; 4] = [
    AccountDriver {
        harness: "claude",
        label: "Claude Code",
        login: AccountLogin::Folder { variable: "CLAUDE_CONFIG_DIR", default: ".claude", layout: FolderLayout::Direct },
        overrides: &CLAUDE_ACCOUNT_OVERRIDES,
    },
    AccountDriver {
        harness: "codex",
        label: "Codex",
        login: AccountLogin::Folder { variable: "CODEX_HOME", default: ".codex", layout: FolderLayout::CodexShadow },
        overrides: &[],
    },
    AccountDriver {
        harness: "opencode",
        label: "OpenCode",
        // OpenCode keeps its logins (and threads) in `<data home>/opencode`.
        // It has no variable of its own for that folder.
        login: AccountLogin::Folder {
            variable: "XDG_DATA_HOME",
            default: ".local/share",
            layout: FolderLayout::DataOverlay { private: "opencode" },
        },
        overrides: &["OPENCODE_API_KEY", "OPENCODE_AUTH_CONTENT"],
    },
    AccountDriver {
        harness: "cursor",
        label: "Cursor",
        // Cursor keeps its CLI login in one global Keychain item
        // (`cursor-access-token`), whatever CURSOR_CONFIG_DIR says.
        login: AccountLogin::ApiKey { variable: "CURSOR_API_KEY" },
        overrides: &["CURSOR_API_KEY", "CURSOR_AUTH_TOKEN"],
    },
];

pub fn driver(harness: &str) -> Option<&'static AccountDriver> {
    DRIVERS.iter().find(|driver| driver.harness == harness)
}

/// Whether `harness` can carry accounts at all.
pub fn supports_accounts(harness: &str) -> bool {
    driver(harness).is_some()
}

/// The variable a folder-based account's folder is handed through.
pub fn home_variable(harness: &str) -> Option<&'static str> {
    match driver(harness)?.login {
        AccountLogin::Folder { variable, .. } => Some(variable),
        AccountLogin::ApiKey { .. } => None,
    }
}

/// A chat's account choice with the reserved spellings of "my own login"
/// (absent, blank, or the harness id) collapsed to `None`.
pub fn normalize_choice(harness: &str, account: Option<&str>) -> Option<String> {
    account
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != harness)
        .map(str::to_owned)
}

/// The validated accounts in a harness's advanced configuration.
pub fn accounts(harness: &str, advanced: &Value) -> Result<Vec<ProviderAccount>, BridgeError> {
    let Some(raw) = advanced.get("accounts") else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    if !supports_accounts(harness) {
        return Err(invalid(format!("{harness} does not support extra accounts")));
    }
    let mut parsed: Vec<ProviderAccount> = serde_json::from_value(raw.clone())
        .map_err(|error| invalid(format!("invalid accounts: {error}")))?;
    let mut seen = BTreeSet::new();
    for account in &mut parsed {
        account.id = account.id.trim().to_owned();
        account.label = account.label.trim().to_owned();
        account.home_path = account.home_path.trim().to_owned();
        account.shadow_home_path = account.shadow_home_path.trim().to_owned();
        validate(harness, account)?;
        if !seen.insert(account.id.clone()) {
            return Err(invalid(format!("account id {} is used twice", account.id)));
        }
    }
    Ok(parsed)
}

/// The account a chat is bound to, or a message the user can act on.
pub fn find(harness: &str, advanced: &Value, id: &str) -> Result<ProviderAccount, BridgeError> {
    accounts(harness, advanced)?
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| {
            invalid(format!(
                "The {} account \"{id}\" is no longer in Settings. Choose another account for this chat.",
                harness_label(harness)
            ))
        })
}

fn validate(harness: &str, account: &ProviderAccount) -> Result<(), BridgeError> {
    let id = account.id.as_str();
    let mut chars = id.chars();
    let well_formed = chars.next().is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !well_formed || id.chars().count() > MAX_ID_CHARS {
        return Err(invalid(format!(
            "account id {id:?} must start with a letter and use only letters, digits, - and _ (64 at most)"
        )));
    }
    if id == harness {
        return Err(invalid(format!("account id {id} is reserved for your own login")));
    }
    if account.label.is_empty() || account.label.chars().count() > MAX_LABEL_CHARS {
        return Err(invalid(format!("account {id} needs a name of 1 to 64 characters")));
    }
    for (field, value) in [("folder", &account.home_path), ("shared folder", &account.shadow_home_path)] {
        if !value.is_empty() && !is_home_relative_or_absolute(value) {
            return Err(invalid(format!(
                "account {id} {field} must start with / or ~ (got {value:?})"
            )));
        }
    }
    let driver = driver(harness).ok_or_else(|| invalid(format!("{harness} does not support extra accounts")))?;
    match driver.login {
        AccountLogin::Folder { layout: FolderLayout::CodexShadow, variable, .. } => {
            if account.home_path.is_empty() && account.shadow_home_path.is_empty() {
                return Err(invalid(format!("account {id} needs a {variable} folder")));
            }
        }
        AccountLogin::Folder { variable, .. } => {
            if account.home_path.is_empty() {
                return Err(invalid(format!("account {id} needs a {variable} folder")));
            }
            if !account.shadow_home_path.is_empty() {
                return Err(invalid(format!(
                    "account {id}: {} has no shared-home arrangement; remove shadowHomePath",
                    driver.label
                )));
            }
        }
        AccountLogin::ApiKey { .. } => {
            if !account.home_path.is_empty() || !account.shadow_home_path.is_empty() {
                return Err(invalid(format!(
                    "account {id}: {} accounts sign in with an API key, not a folder",
                    driver.label
                )));
            }
        }
    }
    Ok(())
}

fn is_home_relative_or_absolute(value: &str) -> bool {
    value == "~" || value.starts_with("~/") || value.starts_with("~\\") || Path::new(value).is_absolute()
}

/// Spawned processes get no shell expansion and would read `~/x` as a
/// relative path (T3 `expandHomePath`).
pub fn expand_home_path(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) {
        return home.join(rest);
    }
    PathBuf::from(value)
}

/// The homes a launch inherits, read once so resolution stays pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHomes {
    pub home: PathBuf,
    pub claude_config_dir: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
}

impl HostHomes {
    pub fn from_env() -> Self {
        let non_empty = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: non_empty("HOME").unwrap_or_else(|| PathBuf::from("/")),
            claude_config_dir: non_empty("CLAUDE_CONFIG_DIR"),
            codex_home: non_empty("CODEX_HOME"),
            xdg_data_home: non_empty("XDG_DATA_HOME"),
        }
    }

    /// The inherited value of a folder variable, if the user set one.
    fn inherited(&self, variable: &str) -> Option<&Path> {
        match variable {
            "CLAUDE_CONFIG_DIR" => self.claude_config_dir.as_deref(),
            "CODEX_HOME" => self.codex_home.as_deref(),
            "XDG_DATA_HOME" => self.xdg_data_home.as_deref(),
            _ => None,
        }
    }
}

/// A folder-based login's folder: the account's, else the inherited
/// variable, else the default under HOME. Inherited variables are not
/// shell-expanded, so a literal `~` stays literal, as it would for the CLI.
fn folder_home(variable: &str, default: &str, account: Option<&ProviderAccount>, host: &HostHomes) -> PathBuf {
    let configured = account.map(|account| account.home_path.as_str()).unwrap_or("");
    if !configured.is_empty() {
        return normalize(&expand_home_path(configured, &host.home));
    }
    host.inherited(variable)
        .map(normalize)
        .unwrap_or_else(|| host.home.join(default))
}

/// The Claude config directory a chat runs in: the account's folder, else the
/// inherited `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_home(account: Option<&ProviderAccount>, host: &HostHomes) -> PathBuf {
    folder_home("CLAUDE_CONFIG_DIR", ".claude", account, host)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexHomeMode {
    /// The account's home is the whole Codex home.
    Direct,
    /// The account's home links the shared one and keeps its own login.
    AuthOverlay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHomeLayout {
    pub mode: CodexHomeMode,
    pub shared_home_path: PathBuf,
    /// What `CODEX_HOME` is set to; `None` leaves it inherited.
    pub effective_home_path: Option<PathBuf>,
    pub continuation_key: String,
}

/// T3 `resolveCodexHomeLayout`, with Bridge's inherited `CODEX_HOME` standing
/// in for an empty home path.
pub fn codex_home_layout(account: Option<&ProviderAccount>, host: &HostHomes) -> CodexHomeLayout {
    let home_path = account.map(|account| account.home_path.as_str()).unwrap_or("");
    let shared_home_path = if home_path.is_empty() {
        host.codex_home
            .as_deref()
            .map(normalize)
            .unwrap_or_else(|| host.home.join(".codex"))
    } else {
        normalize(&expand_home_path(home_path, &host.home))
    };
    let continuation_key = format!("codex:home:{}", shared_home_path.display());
    let shadow = account.map(|account| account.shadow_home_path.as_str()).unwrap_or("");
    if shadow.is_empty() {
        return CodexHomeLayout {
            mode: CodexHomeMode::Direct,
            effective_home_path: (!home_path.is_empty()).then(|| shared_home_path.clone()),
            shared_home_path,
            continuation_key,
        };
    }
    CodexHomeLayout {
        mode: CodexHomeMode::AuthOverlay,
        effective_home_path: Some(normalize(&expand_home_path(shadow, &host.home))),
        shared_home_path,
        continuation_key,
    }
}

/// Which provider threads a chat on this account can resume. A switch between
/// two accounts with the same key keeps the thread; any other switch hands
/// over like a change of harness. `None` for harnesses without accounts.
pub fn continuation_key(
    harness: &str,
    account: Option<&ProviderAccount>,
    host: &HostHomes,
) -> Option<String> {
    let driver = driver(harness)?;
    Some(match driver.login {
        AccountLogin::Folder { layout: FolderLayout::CodexShadow, .. } => codex_home_layout(account, host).continuation_key,
        AccountLogin::Folder { variable, default, .. } => {
            format!("{harness}:home:{}", folder_home(variable, default, account, host).display())
        }
        // T3's default: an instance resumes only its own threads.
        AccountLogin::ApiKey { .. } => {
            format!("{harness}:instance:{}", account.map_or(harness, |account| account.id.as_str()))
        }
    })
}

/// What a launch on an account changes in the agent's environment: the
/// variables it sets and the inherited ones it removes. A value may be an API
/// key, so `Debug` names the variables and never shows a value.
pub struct AccountLaunch {
    harness: &'static str,
    set: Vec<(&'static str, Zeroizing<String>)>,
    removed: &'static [&'static str],
}

impl AccountLaunch {
    pub fn harness(&self) -> &str {
        self.harness
    }

    /// Variables the launch sets, with their values.
    pub fn set(&self) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        self.set.iter().map(|(name, value)| (*name, value.as_str()))
    }

    /// Inherited variables the launch removes.
    pub fn removed(&self) -> &'static [&'static str] {
        self.removed
    }

    pub fn value(&self, variable: &str) -> Option<&str> {
        self.set.iter().find(|(name, _)| *name == variable).map(|(_, value)| value.as_str())
    }
}

impl std::fmt::Debug for AccountLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountLaunch")
            .field("harness", &self.harness)
            .field("set", &self.set.iter().map(|(name, _)| *name).collect::<Vec<_>>())
            .field("removed", &self.removed)
            .finish()
    }
}

/// Make an account ready to launch: link a Codex shadow home or an OpenCode
/// data overlay, or fetch a saved API key.
pub fn prepare_launch(
    harness: &str,
    account: &ProviderAccount,
    host: &HostHomes,
) -> Result<AccountLaunch, BridgeError> {
    let driver = driver(harness).ok_or_else(|| invalid(format!("{harness} does not support extra accounts")))?;
    let value = match driver.login {
        AccountLogin::Folder { layout: FolderLayout::CodexShadow, variable, .. } => {
            let layout = codex_home_layout(Some(account), host);
            materialize_codex_shadow_home(&layout)?;
            let home = layout
                .effective_home_path
                .ok_or_else(|| invalid(format!("account {} has no {variable} folder", account.id)))?;
            (variable, Zeroizing::new(home.to_string_lossy().into_owned()))
        }
        AccountLogin::Folder { layout: FolderLayout::DataOverlay { private }, variable, default } => {
            let overlay = folder_home(variable, default, Some(account), host);
            let base = folder_home(variable, default, None, host);
            materialize_data_overlay(&base, &overlay, private)?;
            (variable, Zeroizing::new(overlay.to_string_lossy().into_owned()))
        }
        AccountLogin::Folder { variable, default, .. } => {
            (variable, Zeroizing::new(folder_home(variable, default, Some(account), host).to_string_lossy().into_owned()))
        }
        AccountLogin::ApiKey { variable } => {
            let key = crate::account_secrets::read(harness, &account.id)?.ok_or_else(|| {
                invalid(format!(
                    "The {} account \"{}\" has no API key yet. Add one in Settings, Coding agents, {}.",
                    driver.label, account.label, driver.label
                ))
            })?;
            (variable, key)
        }
    };
    Ok(AccountLaunch { harness: driver.harness, set: vec![value], removed: driver.overrides })
}

/// Inherited variables that would take a Claude account chat off the login
/// in its config folder: credentials Claude Code prefers over that login, and
/// an endpoint or headers that would send the request (with the account's own
/// token, or an inherited `Authorization`) somewhere else. An account launch
/// drops them all. Generic `HTTPS_PROXY` settings stay: they tunnel TLS and
/// never see the token. Codex reads its login from `CODEX_HOME` alone and
/// needs no such list.
pub const CLAUDE_ACCOUNT_OVERRIDES: [&str; 8] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
];

/// Whether an account launch on `harness` removes the inherited `variable`.
pub fn overridden_by_account(harness: &str, variable: &str) -> bool {
    driver(harness).is_some_and(|driver| driver.overrides.contains(&variable))
}

/// What a launch reads for `key`: the inherited value, unless an account
/// launch removes it. The credential policy judges this view, not the raw
/// environment.
pub fn launch_variable(
    harness: &str,
    has_account: bool,
    key: &str,
    inherited: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if has_account && overridden_by_account(harness, key) {
        None
    } else {
        inherited(key)
    }
}

/// Point a provider launch at an account. Only the agent's own variable is
/// set: Claude keeps `HOME`, since moving it would hide the Keychain login (T3
/// `makeClaudeEnvironment`). Inherited variables that would outrank or
/// redirect the account's login are removed first, so an account's own
/// `CURSOR_API_KEY` replaces an inherited one.
pub fn apply_account(command: &mut std::process::Command, account: Option<&AccountLaunch>) {
    let Some(account) = account else { return };
    for name in account.removed() {
        command.env_remove(name);
    }
    for (name, value) in account.set() {
        command.env(name, value);
    }
}

/// [`apply_account`] for a launch held as a plain variable map (Cursor's
/// agent-protocol client), which can only set: a removed variable is set
/// empty, which every CLI here reads as unset.
pub fn account_env(account: Option<&AccountLaunch>) -> Vec<(&'static str, String)> {
    let Some(account) = account else { return Vec::new() };
    let mut env: Vec<(&'static str, String)> = account
        .removed()
        .iter()
        .filter(|name| account.value(name).is_none())
        .map(|name| (*name, String::new()))
        .collect();
    env.extend(account.set().map(|(name, value)| (name, value.to_owned())));
    env
}

/// T3 `materializeCodexShadowHome`: link every shared entry into the shadow
/// home and keep the login private. Safe to run before every launch.
pub fn materialize_codex_shadow_home(layout: &CodexHomeLayout) -> Result<(), BridgeError> {
    if layout.mode != CodexHomeMode::AuthOverlay {
        return Ok(());
    }
    let Some(shadow) = layout.effective_home_path.as_deref() else {
        return Ok(());
    };
    let shared = layout.shared_home_path.as_path();
    let fs_error = |operation: &str, path: &Path, error: io::Error| {
        invalid(format!(
            "Codex shadow home: could not {operation} {}: {error}",
            path.display()
        ))
    };
    // Compare where the folders really are, not how they are spelled, before
    // touching either: a shadow that resolves to the shared home, inside it,
    // or around it would have setup replace the shared home's own entries
    // with links to themselves.
    let shared_real = resolve(shared).map_err(|e| fs_error("resolve", shared, e))?;
    let shadow_real = resolve(shadow).map_err(|e| fs_error("resolve", shadow, e))?;
    if shadow_real.starts_with(&shared_real) || shared_real.starts_with(&shadow_real) {
        return Err(invalid(format!(
            "Codex shadow home {} must be different from the shared home {}, and neither may contain the other",
            shadow.display(),
            shared.display()
        )));
    }
    // Two chats starting on one new account would both find a link missing,
    // and the second `symlink` would fail. Launches in this process take
    // turns; another process racing it is met by `create_link` and the
    // NotFound-tolerant removals below.
    let lock = shadow_home_lock(&shadow_real);
    let _setup = lock.lock().unwrap_or_else(PoisonError::into_inner);
    fs::create_dir_all(shared).map_err(|e| fs_error("create", shared, e))?;
    fs::create_dir_all(shadow).map_err(|e| fs_error("create", shadow, e))?;
    for directory in KNOWN_SHARED_DIRECTORIES {
        let path = shared.join(directory);
        fs::create_dir_all(&path).map_err(|e| fs_error("create", &path, e))?;
    }
    let mut entries: BTreeSet<String> =
        KNOWN_SHARED_DIRECTORIES.iter().map(|name| (*name).to_owned()).collect();
    for entry in fs::read_dir(shared).map_err(|e| fs_error("read", shared, e))? {
        let name = entry
            .map_err(|e| fs_error("read", shared, e))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if !PRIVATE_ENTRY_NAMES.contains(&name.as_str())
            && !SHADOW_LOCAL_ENTRY_NAMES.contains(&name.as_str())
        {
            entries.insert(name);
        }
    }
    for name in PRIVATE_ENTRY_NAMES {
        if name == "auth.json" {
            continue;
        }
        let path = shadow.join(name);
        if let LinkState::Symlink(_) = link_state(&path).map_err(|e| fs_error("inspect", &path, e))? {
            remove_if_present(fs::remove_file(&path)).map_err(|e| fs_error("remove", &path, e))?;
        }
    }
    for name in &entries {
        ensure_symlink(shared, shadow, name).map_err(|error| match error {
            LinkError::Io(operation, path, error) => fs_error(operation, &path, error),
            LinkError::Conflict(message) => invalid(message),
        })?;
    }
    let auth = shadow.join("auth.json");
    if let LinkState::Symlink(_) = link_state(&auth).map_err(|e| fs_error("inspect", &auth, e))? {
        return Err(invalid(format!(
            "Codex shadow home login {} must be a real file, not a symlink",
            auth.display()
        )));
    }
    Ok(())
}

enum LinkState {
    Missing,
    NotSymlink,
    Symlink(PathBuf),
}

enum LinkError {
    Io(&'static str, PathBuf, io::Error),
    Conflict(String),
}

fn link_state(path: &Path) -> io::Result<LinkState> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LinkState::Missing),
        Err(error) => Err(error),
        Ok(meta) if meta.file_type().is_symlink() => Ok(LinkState::Symlink(fs::read_link(path)?)),
        Ok(_) => Ok(LinkState::NotSymlink),
    }
}

fn ensure_symlink(shared: &Path, shadow: &Path, name: &str) -> Result<(), LinkError> {
    let target = shared.join(name);
    let link = shadow.join(name);
    let io_error = |operation: &'static str, path: &Path| {
        let path = path.to_path_buf();
        move |error| LinkError::Io(operation, path, error)
    };
    let create = || create_link(&target, &link).map_err(io_error("link", &link));
    match link_state(&link).map_err(io_error("inspect", &link))? {
        LinkState::Missing => create(),
        LinkState::NotSymlink => {
            if !REPLACEABLE_SHARED_RUNTIME_ENTRIES.contains(&name) {
                return Err(LinkError::Conflict(format!(
                    "Cannot link Codex shadow home entry {name}: {} already exists and is not a symlink. Move it out of the account folder.",
                    link.display()
                )));
            }
            if name == "thread-writer-locks" {
                // Codex deletes a thread lock on release, so one that is left may
                // still guard a live writer. Keep the local directory until a
                // later setup finds none.
                let held = fs::read_dir(&link)
                    .map_err(io_error("read", &link))?
                    .filter_map(Result::ok)
                    .any(|entry| {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        name.ends_with(".lock") && name != ".coordination.lock"
                    });
                if held {
                    return Ok(());
                }
            }
            let removed = if fs::symlink_metadata(&link).is_ok_and(|meta| meta.is_dir()) {
                fs::remove_dir_all(&link)
            } else {
                fs::remove_file(&link)
            };
            remove_if_present(removed).map_err(io_error("remove", &link))?;
            create()
        }
        LinkState::Symlink(existing) => {
            let resolved = normalize(&link.parent().unwrap_or(shadow).join(existing));
            if resolved != target {
                remove_if_present(fs::remove_file(&link)).map_err(io_error("remove", &link))?;
                create()?;
            }
            Ok(())
        }
    }
}

/// Where `path` really is: its deepest existing ancestor resolved through
/// symlinks, with the missing rest appended, so nothing has to exist (or be
/// created) to ask.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(real) => return Ok(missing.iter().rev().fold(real, |path, part| path.join(part))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
                    return Err(error);
                };
                missing.push(name.to_owned());
                existing = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Link every entry of the user's data home into an account's overlay except
/// the agent's own `private` folder, which stays real so the account keeps its
/// own logins and threads. An entry the overlay already holds for real (a tool
/// wrote it there) is left alone: nothing here deletes data.
pub fn materialize_data_overlay(base: &Path, overlay: &Path, private: &str) -> Result<(), BridgeError> {
    let fs_error = |operation: &str, path: &Path, error: io::Error| {
        invalid(format!("Account data folder: could not {operation} {}: {error}", path.display()))
    };
    let base_real = resolve(base).map_err(|e| fs_error("resolve", base, e))?;
    let overlay_real = resolve(overlay).map_err(|e| fs_error("resolve", overlay, e))?;
    if overlay_real.starts_with(&base_real) || base_real.starts_with(&overlay_real) {
        return Err(invalid(format!(
            "Account data folder {} must be different from your data home {}, and neither may contain the other",
            overlay.display(),
            base.display()
        )));
    }
    let lock = shadow_home_lock(&overlay_real);
    let _setup = lock.lock().unwrap_or_else(PoisonError::into_inner);
    fs::create_dir_all(base).map_err(|e| fs_error("create", base, e))?;
    let private_dir = overlay.join(private);
    fs::create_dir_all(&private_dir).map_err(|e| fs_error("create", &private_dir, e))?;
    for entry in fs::read_dir(base).map_err(|e| fs_error("read", base, e))? {
        let name = entry.map_err(|e| fs_error("read", base, e))?.file_name();
        let text = name.to_string_lossy();
        if text == private || text == ".DS_Store" {
            continue;
        }
        let target = base.join(&name);
        let link = overlay.join(&name);
        match link_state(&link).map_err(|e| fs_error("inspect", &link, e))? {
            LinkState::NotSymlink => {}
            LinkState::Missing => create_link(&target, &link).map_err(|e| fs_error("link", &link, e))?,
            LinkState::Symlink(existing) => {
                if normalize(&link.parent().unwrap_or(overlay).join(existing)) != target {
                    remove_if_present(fs::remove_file(&link)).map_err(|e| fs_error("remove", &link, e))?;
                    create_link(&target, &link).map_err(|e| fs_error("link", &link, e))?;
                }
            }
        }
    }
    Ok(())
}

/// The setup lock for one shadow home, shared by every launch in this process.
fn shadow_home_lock(shadow: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(normalize(shadow))
        .or_default()
        .clone()
}

/// Link `link` to `target`, accepting a link someone else made a moment ago
/// if it points where this one would.
fn create_link(target: &Path, link: &Path) -> io::Result<()> {
    match std::os::unix::fs::symlink(target, link) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let made = fs::read_link(link).map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, error.to_string()))?;
            if normalize(&link.parent().unwrap_or(link).join(made)) == target {
                Ok(())
            } else {
                Err(error)
            }
        }
        other => other,
    }
}

/// A removal another launch already made is not a failure.
fn remove_if_present(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Lexical `path.resolve`: drops `.` and folds `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn harness_label(harness: &str) -> &str {
    driver(harness).map_or(harness, |driver| driver.label)
}

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host(home: &Path) -> HostHomes {
        HostHomes { home: home.to_path_buf(), claude_config_dir: None, codex_home: None, xdg_data_home: None }
    }

    /// A folder account's launch, or none for an agent without accounts.
    fn folder_launch(harness: &str, folder: Option<&Path>) -> Option<AccountLaunch> {
        let folder = folder?;
        supports_accounts(harness).then(|| {
            prepare_launch(harness, &account("acct", &folder.display().to_string(), ""), &host(Path::new("/home/ada"))).unwrap()
        })
    }

    fn account(id: &str, home: &str, shadow: &str) -> ProviderAccount {
        ProviderAccount {
            id: id.into(),
            label: id.to_uppercase(),
            home_path: home.into(),
            shadow_home_path: shadow.into(),
        }
    }

    #[test]
    fn expands_home_relative_paths() {
        let home = Path::new("/home/ada");
        assert_eq!(expand_home_path("~", home), PathBuf::from("/home/ada"));
        assert_eq!(expand_home_path("~/.account", home), PathBuf::from("/home/ada/.account"));
        assert_eq!(expand_home_path("~\\.account", home), PathBuf::from("/home/ada/.account"));
        assert_eq!(expand_home_path("/abs/x", home), PathBuf::from("/abs/x"));
        assert_eq!(expand_home_path("~other/x", home), PathBuf::from("~other/x"));
        assert_eq!(expand_home_path("", home), PathBuf::from(""));
    }

    #[test]
    fn parses_valid_accounts_and_ignores_other_advanced_keys() {
        let advanced = json!({
            "workerCredentialSource": "auto",
            "accounts": [{"id": " personal ", "label": " Personal ", "homePath": " ~/.claude_personal "}],
        });
        let parsed = accounts("claude", &advanced).unwrap();
        assert_eq!(parsed, vec![ProviderAccount {
            id: "personal".into(),
            label: "Personal".into(),
            home_path: "~/.claude_personal".into(),
            shadow_home_path: String::new(),
        }]);
        assert!(accounts("claude", &json!({})).unwrap().is_empty());
        assert!(accounts("claude", &json!({"accounts": null})).unwrap().is_empty());
        assert_eq!(find("claude", &advanced, "personal").unwrap().label, "Personal");
        let missing = find("claude", &advanced, "work").unwrap_err().to_string();
        assert!(missing.contains("Claude Code account \"work\""), "{missing}");
    }

    #[test]
    fn rejects_bad_ids_duplicates_and_reserved_harness_id() {
        let reject = |accounts_value: Value| {
            accounts("codex", &json!({"accounts": accounts_value})).unwrap_err().to_string()
        };
        assert!(reject(json!([{"id": "1st", "label": "x", "homePath": "~/.c"}])).contains("must start with a letter"));
        assert!(reject(json!([{"id": "a b", "label": "x", "homePath": "~/.c"}])).contains("must start with a letter"));
        assert!(reject(json!([{"id": "a".repeat(65), "label": "x", "homePath": "~/.c"}])).contains("64 at most"));
        assert!(reject(json!([{"id": "codex", "label": "x", "homePath": "~/.c"}])).contains("reserved"));
        assert!(reject(json!([{"id": "a", "label": " ", "homePath": "~/.c"}])).contains("needs a name"));
        assert!(reject(json!([{"id": "a", "label": "x", "homePath": "rel/path"}])).contains("must start with / or ~"));
        assert!(reject(json!([
            {"id": "a", "label": "x", "homePath": "~/.c1"},
            {"id": "a", "label": "y", "homePath": "~/.c2"},
        ])).contains("used twice"));
        assert!(reject(json!([{"id": "a", "label": "x", "homePath": "~/.c", "extra": 1}])).contains("invalid accounts"));
        assert!(reject(json!({"id": "a"})).contains("invalid accounts"));
    }

    #[test]
    fn rejects_shadow_home_on_claude_and_empty_claude_home() {
        let shadow = accounts("claude", &json!({"accounts": [{"id": "a", "label": "A", "homePath": "~/.c", "shadowHomePath": "~/.s"}]}));
        assert!(shadow.unwrap_err().to_string().contains("no shared-home"));
        let empty = accounts("claude", &json!({"accounts": [{"id": "a", "label": "A"}]}));
        assert!(empty.unwrap_err().to_string().contains("CLAUDE_CONFIG_DIR"));
        let empty_codex = accounts("codex", &json!({"accounts": [{"id": "a", "label": "A"}]}));
        assert!(empty_codex.unwrap_err().to_string().contains("CODEX_HOME"));
        // A Codex shadow account may leave the shared home inherited.
        assert_eq!(accounts("codex", &json!({"accounts": [{"id": "a", "label": "A", "shadowHomePath": "~/.s"}]})).unwrap().len(), 1);
    }

    #[test]
    fn rejects_accounts_on_unsupported_harness() {
        let error = accounts("grok", &json!({"accounts": [{"id": "a", "label": "A", "homePath": "~/.o"}]}));
        assert!(error.unwrap_err().to_string().contains("does not support extra accounts"));
        // An absent key is not an account list, for any harness.
        assert!(accounts("grok", &json!({"visibleModels": []})).unwrap().is_empty());
        assert_eq!(normalize_choice("claude", Some("claude")), None);
        assert_eq!(normalize_choice("claude", Some("  ")), None);
        assert_eq!(normalize_choice("claude", None), None);
        assert_eq!(normalize_choice("claude", Some(" personal ")), Some("personal".into()));
        assert_eq!(home_variable("claude"), Some("CLAUDE_CONFIG_DIR"));
        assert_eq!(home_variable("codex"), Some("CODEX_HOME"));
        assert_eq!(home_variable("opencode"), Some("XDG_DATA_HOME"));
        assert_eq!(home_variable("cursor"), None, "Cursor accounts sign in with a key");
        assert_eq!(home_variable("grok"), None);
    }

    #[test]
    fn claude_home_and_continuation_key() {
        let home = Path::new("/home/ada");
        let mut host = host(home);
        assert_eq!(claude_home(None, &host), PathBuf::from("/home/ada/.claude"));
        host.claude_config_dir = Some(PathBuf::from("/cfg/claude/./x/.."));
        assert_eq!(claude_home(None, &host), PathBuf::from("/cfg/claude"));
        let personal = account("personal", "~/.claude_personal", "");
        assert_eq!(claude_home(Some(&personal), &host), PathBuf::from("/home/ada/.claude_personal"));
        assert_eq!(
            continuation_key("claude", Some(&personal), &host).as_deref(),
            Some("claude:home:/home/ada/.claude_personal")
        );
        assert_ne!(continuation_key("claude", None, &host), continuation_key("claude", Some(&personal), &host));
        assert_eq!(continuation_key("grok", None, &host), None);
        assert_eq!(prepare_launch("claude", &personal, &host).unwrap().value("CLAUDE_CONFIG_DIR"), Some("/home/ada/.claude_personal"));
    }

    #[test]
    fn codex_direct_and_shadow_layouts_share_continuation_keys() {
        let home = Path::new("/home/ada");
        let host = host(home);
        let own = codex_home_layout(None, &host);
        assert_eq!(own.mode, CodexHomeMode::Direct);
        assert_eq!(own.shared_home_path, PathBuf::from("/home/ada/.codex"));
        assert_eq!(own.effective_home_path, None);

        let shadow = account("personal", "", "~/.codex_personal");
        let overlay = codex_home_layout(Some(&shadow), &host);
        assert_eq!(overlay.mode, CodexHomeMode::AuthOverlay);
        assert_eq!(overlay.effective_home_path, Some(PathBuf::from("/home/ada/.codex_personal")));
        assert_eq!(overlay.continuation_key, own.continuation_key);

        let separate = account("work", "~/.codex_work", "");
        let direct = codex_home_layout(Some(&separate), &host);
        assert_eq!(direct.mode, CodexHomeMode::Direct);
        assert_eq!(direct.effective_home_path, Some(PathBuf::from("/home/ada/.codex_work")));
        assert_ne!(direct.continuation_key, own.continuation_key);

        let inherited = HostHomes { codex_home: Some(PathBuf::from("/cfg/codex")), ..host };
        assert_eq!(codex_home_layout(Some(&shadow), &inherited).shared_home_path, PathBuf::from("/cfg/codex"));
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn overlay(shared: &Path, shadow: &Path) -> CodexHomeLayout {
        CodexHomeLayout {
            mode: CodexHomeMode::AuthOverlay,
            shared_home_path: shared.to_path_buf(),
            effective_home_path: Some(shadow.to_path_buf()),
            continuation_key: format!("codex:home:{}", shared.display()),
        }
    }

    #[test]
    fn materializes_shadow_home_with_shared_links_and_private_auth() {
        let shared_dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let shared = shared_dir.path();
        let shadow = root.path().join("shadow");
        fs::create_dir_all(shared.join("sessions")).unwrap();
        write(&shared.join("config.toml"), "model = \"gpt-5-codex\"\n");
        write(&shared.join("models_cache.json"), "{\"models\":[\"shared\"]}\n");
        write(&shared.join("auth.json"), "{\"shared\":true}\n");
        write(&shared.join("auth.lock"), "");
        write(&shadow.join("auth.json"), "{\"shadow\":true}\n");
        write(&shadow.join("auth.lock"), "");
        std::os::unix::fs::symlink(shared.join("models_cache.json"), shadow.join("models_cache.json")).unwrap();

        materialize_codex_shadow_home(&overlay(shared, &shadow)).unwrap();

        assert_eq!(fs::read_link(shadow.join("sessions")).unwrap(), shared.join("sessions"));
        assert_eq!(fs::read_link(shadow.join("config.toml")).unwrap(), shared.join("config.toml"));
        assert_eq!(fs::read_link(shadow.join("mcp-oauth-locks")).unwrap(), shared.join("mcp-oauth-locks"));
        assert!(!shadow.join("models_cache.json").exists(), "a linked private cache is removed");
        assert!(fs::symlink_metadata(shadow.join("auth.json")).unwrap().file_type().is_file());
        assert_eq!(fs::read_to_string(shadow.join("auth.json")).unwrap(), "{\"shadow\":true}\n");
        assert!(fs::symlink_metadata(shadow.join("auth.lock")).unwrap().file_type().is_file());
        assert!(shared.join("thread-writer-locks").is_dir());

        // Idempotent, and a link pointing elsewhere is repointed.
        fs::remove_file(shadow.join("config.toml")).unwrap();
        std::os::unix::fs::symlink("/elsewhere/config.toml", shadow.join("config.toml")).unwrap();
        materialize_codex_shadow_home(&overlay(shared, &shadow)).unwrap();
        assert_eq!(fs::read_link(shadow.join("config.toml")).unwrap(), shared.join("config.toml"));
    }

    #[test]
    fn shadow_home_keeps_local_entries_local() {
        let shared_dir = tempfile::tempdir().unwrap();
        let shadow_dir = tempfile::tempdir().unwrap();
        let (shared, shadow) = (shared_dir.path(), shadow_dir.path());
        write(&shared.join(".DS_Store"), "shared Finder metadata");
        write(&shared.join("log/codex.log"), "shared");
        write(&shadow.join(".DS_Store"), "shadow Finder metadata");
        write(&shadow.join("auth.json"), "{}");
        materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap();
        materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap();
        assert_eq!(fs::read_to_string(shadow.join(".DS_Store")).unwrap(), "shadow Finder metadata");
        assert_eq!(fs::read_to_string(shared.join(".DS_Store")).unwrap(), "shared Finder metadata");
        assert!(!shadow.join("log").exists());
    }

    #[test]
    fn shadow_home_replaces_local_runtime_locks_unless_a_writer_holds_one() {
        let shared_dir = tempfile::tempdir().unwrap();
        let shadow_dir = tempfile::tempdir().unwrap();
        let (shared, shadow) = (shared_dir.path(), shadow_dir.path());
        fs::create_dir_all(shadow.join("mcp-oauth-locks")).unwrap();
        write(&shadow.join("thread-writer-locks/t1.lock"), "");
        materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap();
        assert_eq!(fs::read_link(shadow.join("mcp-oauth-locks")).unwrap(), shared.join("mcp-oauth-locks"));
        assert!(fs::symlink_metadata(shadow.join("thread-writer-locks")).unwrap().is_dir(), "a held lock stays local");
        fs::remove_file(shadow.join("thread-writer-locks/t1.lock")).unwrap();
        materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap();
        assert_eq!(fs::read_link(shadow.join("thread-writer-locks")).unwrap(), shared.join("thread-writer-locks"));
    }

    #[test]
    fn shadow_home_refuses_real_file_blocking_a_shared_link() {
        let shared_dir = tempfile::tempdir().unwrap();
        let shadow_dir = tempfile::tempdir().unwrap();
        let (shared, shadow) = (shared_dir.path(), shadow_dir.path());
        write(&shared.join("config.toml"), "shared");
        write(&shadow.join("config.toml"), "copied by hand");
        let error = materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap_err().to_string();
        assert!(error.contains("config.toml") && error.contains("not a symlink"), "{error}");
        assert_eq!(fs::read_to_string(shadow.join("config.toml")).unwrap(), "copied by hand");
    }

    #[test]
    fn shadow_home_refuses_symlinked_auth() {
        let shared_dir = tempfile::tempdir().unwrap();
        let shadow_dir = tempfile::tempdir().unwrap();
        let (shared, shadow) = (shared_dir.path(), shadow_dir.path());
        write(&shared.join("auth.json"), "{}");
        std::os::unix::fs::symlink(shared.join("auth.json"), shadow.join("auth.json")).unwrap();
        let error = materialize_codex_shadow_home(&overlay(shared, shadow)).unwrap_err().to_string();
        assert!(error.contains("must be a real file"), "{error}");
    }

    #[test]
    fn shadow_home_equal_to_shared_home_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = materialize_codex_shadow_home(&overlay(dir.path(), dir.path())).unwrap_err().to_string();
        assert!(error.contains("must be different"), "{error}");
        // A direct layout never touches the disk.
        let direct = CodexHomeLayout { mode: CodexHomeMode::Direct, ..overlay(dir.path(), &dir.path().join("x")) };
        materialize_codex_shadow_home(&direct).unwrap();
        assert!(!dir.path().join("x").exists());
    }

    #[test]
    fn apply_account_home_sets_only_the_provider_variable() {
        let envs = |harness: &str, home: Option<&Path>| {
            let mut command = std::process::Command::new("true");
            apply_account(&mut command, folder_launch(harness, home).as_ref());
            command
                .get_envs()
                .map(|(key, value)| (key.to_string_lossy().into_owned(), value.map(|v| v.to_string_lossy().into_owned())))
                .collect::<Vec<_>>()
        };
        let home = Path::new("/home/ada/.claude_personal");
        let claude = envs("claude", Some(home));
        let set: Vec<_> = claude.iter().filter(|(_, value)| value.is_some()).cloned().collect();
        assert_eq!(set, vec![("CLAUDE_CONFIG_DIR".into(), Some("/home/ada/.claude_personal".into()))]);
        let mut removed: Vec<_> = claude.iter().filter(|(_, value)| value.is_none()).map(|(key, _)| key.clone()).collect();
        let mut expected: Vec<String> = CLAUDE_ACCOUNT_OVERRIDES.iter().map(|name| (*name).to_owned()).collect();
        removed.sort();
        expected.sort();
        assert_eq!(removed, expected, "HOME and everything else is left alone");
        assert_eq!(envs("codex", Some(home)), vec![("CODEX_HOME".into(), Some("/home/ada/.claude_personal".into()))]);
        assert!(envs("claude", None).is_empty(), "no account inherits the user's own login");
        assert!(envs("grok", Some(home)).is_empty());
    }

    #[test]
    fn explicit_claude_account_drops_inherited_credentials() {
        let home = tempfile::tempdir().unwrap();
        let run = |harness: &str, account: Option<&Path>| {
            // Stands in for what the daemon inherits; `env` prints what the
            // provider would actually see.
            let mut command = std::process::Command::new("/usr/bin/env");
            command
                .env("ANTHROPIC_API_KEY", "sk-ant-inherited")
                .env("CLAUDE_CODE_OAUTH_TOKEN", "inherited-oauth")
                .env("ANTHROPIC_AUTH_TOKEN", "inherited-bearer")
                .env("CLAUDE_CODE_USE_BEDROCK", "1")
                .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/inherited-proxy")
                .env("ANTHROPIC_CUSTOM_HEADERS", "Authorization: Bearer inherited-header");
            apply_account(&mut command, folder_launch(harness, account).as_ref());
            String::from_utf8(command.output().unwrap().stdout).unwrap()
        };
        let with_account = run("claude", Some(home.path()));
        for name in CLAUDE_ACCOUNT_OVERRIDES {
            assert!(!with_account.lines().any(|line| line.starts_with(&format!("{name}="))), "{name} survived:\n{with_account}");
        }
        assert!(with_account.contains(&format!("CLAUDE_CONFIG_DIR={}", home.path().display())));
        // The user's own login keeps whatever it was configured with.
        let own = run("claude", None);
        assert!(own.contains("ANTHROPIC_API_KEY=sk-ant-inherited") && own.contains("CLAUDE_CODE_OAUTH_TOKEN=inherited-oauth"));
        // Codex reads its login from CODEX_HOME alone.
        assert!(run("codex", Some(home.path())).contains("ANTHROPIC_API_KEY=sk-ant-inherited"));
    }

    /// A listener that records each request's head and answers `reply`.
    fn recorder(reply: &'static [u8]) -> (u16, Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut head = Vec::new();
                let mut buffer = [0u8; 4096];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => head.extend_from_slice(&buffer[..read]),
                    }
                }
                log.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
                let _ = stream.write_all(reply);
            }
        });
        (port, seen)
    }

    /// Runs the real `claude` (skipped when it is not installed) with
    /// synthetic credentials only: a throwaway HOME and config folder holding
    /// a fake subscription login, an inherited proxy endpoint and
    /// `Authorization` header pointing at a local recorder, and a stand-in
    /// HTTPS proxy that sees only which host traffic is tunnelled to.
    #[test]
    fn an_account_launch_never_reaches_an_inherited_endpoint() {
        if std::process::Command::new("claude").arg("--version").output().is_err() {
            eprintln!("skipping: claude is not installed");
            return;
        }
        const UNAUTHORIZED: &[u8] = b"HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: 70\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"x\"}}";
        const NO_TUNNEL: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        // Each run ends after its first messages request fails (no retries),
        // so what the recorders hold afterwards is everything it sent.
        let run = |account: bool, inherited_header: bool| {
            let (home, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let expires = (std::time::SystemTime::now() + std::time::Duration::from_secs(86_400))
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
            fs::write(
                folder.path().join(".credentials.json"),
                format!("{{\"claudeAiOauth\":{{\"accessToken\":\"sk-ant-oat01-SYNTHETIC-ACCOUNT\",\"refreshToken\":\"sk-ant-ort01-SYNTHETIC\",\"expiresAt\":{expires},\"scopes\":[\"user:inference\",\"user:profile\"],\"subscriptionType\":\"max\"}}}}"),
            ).unwrap();
            let (endpoint_port, endpoint) = recorder(UNAUTHORIZED);
            let (proxy_port, proxy) = recorder(NO_TUNNEL);
            let mut command = std::process::Command::new("claude");
            command
                .args(["-p", "hi", "--max-turns", "1"])
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", home.path())
                .env("CLAUDE_CONFIG_DIR", folder.path())
                .env("HTTPS_PROXY", format!("http://127.0.0.1:{proxy_port}"))
                .env("NO_PROXY", "127.0.0.1,localhost")
                .env("DISABLE_TELEMETRY", "1")
                .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
                .env("CLAUDE_CODE_MAX_RETRIES", "0")
                // What the daemon inherited from the user's shell.
                .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{endpoint_port}"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if inherited_header {
                command.env("ANTHROPIC_CUSTOM_HEADERS", "Authorization: Bearer SYNTHETIC-INHERITED-BEARER");
            }
            if account {
                apply_account(&mut command, folder_launch("claude", Some(folder.path())).as_ref());
            }
            let mut child = command.spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while std::time::Instant::now() < deadline && child.try_wait().unwrap().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert!(child.try_wait().unwrap().is_some(), "claude did not finish within a minute");
            let _ = child.wait();
            let seen = (endpoint.lock().unwrap().clone(), proxy.lock().unwrap().clone());
            seen
        };
        // Controls: without the account launch the inherited endpoint wins,
        // carrying the inherited header, or with only a base URL the account's
        // own token. That is what makes the account runs a real check.
        for (header, token) in [(true, "Bearer SYNTHETIC-INHERITED-BEARER"), (false, "Bearer sk-ant-oat01-SYNTHETIC-ACCOUNT")] {
            let (endpoint, _) = run(false, header);
            assert!(
                endpoint.iter().any(|head| head.starts_with("POST /v1/messages") && head.contains(token)),
                "the control run should send {token} to the inherited endpoint: {endpoint:?}"
            );
            // The account launch goes to Anthropic, tunnelled, and never to
            // the inherited endpoint with either token.
            let (endpoint, proxy) = run(true, header);
            assert!(proxy.iter().any(|head| head.starts_with("CONNECT api.anthropic.com:443")), "{proxy:?}");
            assert!(endpoint.is_empty(), "an account launch reached the inherited endpoint: {endpoint:?}");
        }
    }

    #[test]
    fn the_credential_policy_judges_the_account_launch_environment() {
        use crate::credential_policy::{admit_claude_environment, CredentialPolicy};
        let inherited = |key: &str| (key == "ANTHROPIC_API_KEY").then(|| "sk-ant-inherited".to_owned());
        let view = |account: bool| move |key: &str| launch_variable("claude", account, key, inherited);
        // The user's own login runs on the inherited key, which API-key-only admits.
        assert!(admit_claude_environment(CredentialPolicy::ApiKeyOnly, view(false)).is_ok());
        // An account runs on its subscription login, which that policy refuses.
        let refused = admit_claude_environment(CredentialPolicy::ApiKeyOnly, view(true)).unwrap_err().to_string();
        assert!(refused.contains("a subscription login"), "{refused}");
        assert!(admit_claude_environment(CredentialPolicy::UserManaged, view(true)).is_ok());
        assert_eq!(launch_variable("codex", true, "ANTHROPIC_API_KEY", inherited).as_deref(), Some("sk-ant-inherited"));
    }

    #[test]
    fn concurrent_first_launches_share_one_shadow_home() {
        for _ in 0..40 {
            let shared_dir = tempfile::tempdir().unwrap();
            let root = tempfile::tempdir().unwrap();
            let shared = shared_dir.path().to_path_buf();
            let shadow = root.path().join("shadow");
            write(&shared.join("config.toml"), "model = \"x\"\n");
            write(&shared.join("auth.json"), "{}");
            let start = Arc::new(std::sync::Barrier::new(8));
            let results: Vec<_> = (0..8)
                .map(|_| {
                    let (start, layout) = (start.clone(), overlay(&shared, &shadow));
                    std::thread::spawn(move || {
                        start.wait();
                        materialize_codex_shadow_home(&layout)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            assert!(results.iter().all(Result::is_ok), "{results:?}");
            assert_eq!(fs::read_link(shadow.join("sessions")).unwrap(), shared.join("sessions"));
            assert_eq!(fs::read_link(shadow.join("config.toml")).unwrap(), shared.join("config.toml"));
        }
    }

    #[test]
    fn a_link_created_by_another_launch_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("shared/sessions");
        let link = dir.path().join("sessions");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        create_link(&target, &link).unwrap();
        // One pointing elsewhere is still a conflict.
        let other = dir.path().join("elsewhere");
        assert_eq!(create_link(&other, &link).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        // So is a real file in the way.
        let file = dir.path().join("config.toml");
        fs::write(&file, "x").unwrap();
        assert!(create_link(&target, &file).is_err());
        remove_if_present(fs::remove_file(dir.path().join("gone"))).unwrap();
    }

    #[test]
    fn a_shadow_that_aliases_the_shared_home_is_refused_untouched() {
        let root = tempfile::tempdir().unwrap();
        let shared = root.path().join("codex");
        write(&shared.join(".sqlite-maintenance.lock"), "held");
        write(&shared.join("config.toml"), "model = \"x\"\n");
        let lock_is_intact = || {
            assert!(fs::symlink_metadata(shared.join(".sqlite-maintenance.lock")).unwrap().file_type().is_file());
            assert_eq!(fs::read_to_string(shared.join(".sqlite-maintenance.lock")).unwrap(), "held");
            assert!(fs::symlink_metadata(shared.join("config.toml")).unwrap().file_type().is_file());
        };
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&shared, &alias).unwrap();
        let through_alias = root.path().join("through");
        std::os::unix::fs::symlink(&shared, &through_alias).unwrap();
        for shadow in [
            alias.clone(),                    // a symlink to the shared home
            through_alias.join("personal"),   // inside it, through a symlinked parent
            shared.join("personal"),          // inside it, plainly
            root.path().to_path_buf(),        // around it
        ] {
            let error = materialize_codex_shadow_home(&overlay(&shared, &shadow)).unwrap_err().to_string();
            assert!(error.contains("must be different"), "{}: {error}", shadow.display());
            lock_is_intact();
        }
        assert!(!shared.join("personal").exists(), "a refused setup creates nothing");
        assert_eq!(resolve(&through_alias.join("a/b")).unwrap(), fs::canonicalize(&shared).unwrap().join("a/b"));
    }

    #[test]
    fn the_driver_table_names_every_supported_agent() {
        let harnesses: Vec<_> = DRIVERS.iter().map(|driver| driver.harness).collect();
        assert_eq!(harnesses, ["claude", "codex", "opencode", "cursor"]);
        for driver in DRIVERS {
            assert!(crate::agent_config::VALID_HARNESS_IDS.contains(&driver.harness), "{} is not a harness", driver.harness);
            assert!(supports_accounts(driver.harness));
        }
        assert!(!supports_accounts("grok") && !supports_accounts("pi") && !supports_accounts("bridge"));
        assert!(overridden_by_account("cursor", "CURSOR_AUTH_TOKEN"));
        assert!(overridden_by_account("opencode", "OPENCODE_API_KEY"));
        assert!(!overridden_by_account("codex", "OPENAI_API_KEY"));
    }

    #[test]
    fn opencode_and_cursor_accounts_validate_by_login_kind() {
        let check = |harness: &str, account: Value| accounts(harness, &json!({"accounts": [account]}));
        assert_eq!(check("opencode", json!({"id": "go", "label": "Go", "homePath": "~/.opencode_go"})).unwrap().len(), 1);
        assert!(check("opencode", json!({"id": "go", "label": "Go"})).unwrap_err().to_string().contains("XDG_DATA_HOME"));
        assert!(check("opencode", json!({"id": "go", "label": "Go", "homePath": "~/.o", "shadowHomePath": "~/.s"})).unwrap_err().to_string().contains("no shared-home"));
        assert_eq!(check("cursor", json!({"id": "work", "label": "Work"})).unwrap().len(), 1);
        assert!(check("cursor", json!({"id": "work", "label": "Work", "homePath": "~/.cursor_work"})).unwrap_err().to_string().contains("API key"));
        assert!(check("cursor", json!({"id": "cursor", "label": "Mine"})).unwrap_err().to_string().contains("reserved"));
    }

    #[test]
    fn an_opencode_overlay_keeps_its_data_private_and_links_the_rest() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join(".local/share");
        write(&base.join("opencode/opencode.db"), "the user's own logins");
        write(&base.join("mise/installs/node"), "22");
        fs::create_dir_all(base.join("uv")).unwrap();
        let mut host = host(home.path());
        host.xdg_data_home = None;
        let go = account("go", "~/.opencode_go", "");
        let launch = prepare_launch("opencode", &go, &host).unwrap();
        let overlay = home.path().join(".opencode_go");
        assert_eq!(launch.value("XDG_DATA_HOME"), Some(overlay.to_str().unwrap()));
        assert_eq!(launch.removed(), ["OPENCODE_API_KEY", "OPENCODE_AUTH_CONTENT"]);
        // The account's own OpenCode data is real and empty; never the user's.
        assert!(fs::symlink_metadata(overlay.join("opencode")).unwrap().is_dir());
        assert!(!overlay.join("opencode/opencode.db").exists());
        // Everything else is the user's, so mise and uv keep working in tools.
        assert_eq!(fs::read_link(overlay.join("mise")).unwrap(), base.join("mise"));
        assert_eq!(fs::read_to_string(overlay.join("mise/installs/node")).unwrap(), "22");
        assert_eq!(fs::read_link(overlay.join("uv")).unwrap(), base.join("uv"));
        // Idempotent; a new entry in the data home is linked next time; a real
        // entry a tool wrote into the overlay is left alone.
        fs::create_dir_all(base.join("pnpm")).unwrap();
        write(&overlay.join("tool-local/state"), "kept");
        fs::create_dir_all(base.join("tool-local")).unwrap();
        prepare_launch("opencode", &go, &host).unwrap();
        assert_eq!(fs::read_link(overlay.join("pnpm")).unwrap(), base.join("pnpm"));
        assert_eq!(fs::read_to_string(overlay.join("tool-local/state")).unwrap(), "kept");
        // An inherited XDG_DATA_HOME is the data home it links to.
        let custom = home.path().join("data");
        fs::create_dir_all(custom.join("gh")).unwrap();
        host.xdg_data_home = Some(custom.clone());
        prepare_launch("opencode", &account("work", "~/.opencode_work", ""), &host).unwrap();
        assert_eq!(fs::read_link(home.path().join(".opencode_work/gh")).unwrap(), custom.join("gh"));
    }

    #[test]
    fn an_opencode_overlay_that_aliases_the_data_home_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join(".local/share");
        write(&base.join("opencode/opencode.db"), "the user's own logins");
        let alias = home.path().join("alias");
        std::os::unix::fs::symlink(&base, &alias).unwrap();
        for folder in ["~/alias", "~/.local/share", "~/.local/share/opencode-go", "~/.local"] {
            let error = prepare_launch("opencode", &account("go", folder, ""), &host(home.path())).unwrap_err().to_string();
            assert!(error.contains("must be different"), "{folder}: {error}");
        }
        assert_eq!(fs::read_to_string(base.join("opencode/opencode.db")).unwrap(), "the user's own logins");
        assert!(!base.join("opencode-go").exists());
    }

    #[test]
    fn a_cursor_account_launch_hands_over_its_key_and_hides_it() {
        crate::account_secrets::save("cursor", "work-launch", "key_cursor_work").unwrap();
        let work = ProviderAccount { id: "work-launch".into(), label: "Work".into(), home_path: String::new(), shadow_home_path: String::new() };
        let launch = prepare_launch("cursor", &work, &host(Path::new("/home/ada"))).unwrap();
        assert_eq!(launch.value("CURSOR_API_KEY"), Some("key_cursor_work"));
        let debug = format!("{launch:?}");
        assert!(debug.contains("CURSOR_API_KEY") && !debug.contains("key_cursor_work"), "{debug}");
        // The plain-map form blanks an inherited auth token and sets the key once.
        let env = account_env(Some(&launch));
        assert_eq!(env, vec![("CURSOR_AUTH_TOKEN", String::new()), ("CURSOR_API_KEY", "key_cursor_work".to_owned())]);
        // A process sees the account's key, never an inherited one.
        let mut command = std::process::Command::new("/usr/bin/env");
        command.env("CURSOR_API_KEY", "inherited").env("CURSOR_AUTH_TOKEN", "inherited-token");
        apply_account(&mut command, Some(&launch));
        let seen = String::from_utf8(command.output().unwrap().stdout).unwrap();
        assert!(seen.lines().any(|line| line == "CURSOR_API_KEY=key_cursor_work"), "{seen}");
        assert!(!seen.contains("inherited"), "{seen}");
        crate::account_secrets::delete("cursor", "work-launch").unwrap();
    }

    #[test]
    fn a_cursor_account_without_a_key_names_where_to_add_one() {
        let personal = ProviderAccount { id: "no-key-yet".into(), label: "Personal".into(), home_path: String::new(), shadow_home_path: String::new() };
        let error = prepare_launch("cursor", &personal, &host(Path::new("/home/ada"))).unwrap_err().to_string();
        assert!(error.contains("Cursor account \"Personal\" has no API key yet") && error.contains("Settings"), "{error}");
    }

    #[test]
    fn continuation_keys_follow_the_login_kind() {
        let host = host(Path::new("/home/ada"));
        let go = account("go", "~/.opencode_go", "");
        assert_eq!(continuation_key("opencode", None, &host).as_deref(), Some("opencode:home:/home/ada/.local/share"));
        assert_eq!(continuation_key("opencode", Some(&go), &host).as_deref(), Some("opencode:home:/home/ada/.opencode_go"));
        let work = ProviderAccount { id: "work".into(), label: "Work".into(), home_path: String::new(), shadow_home_path: String::new() };
        assert_eq!(continuation_key("cursor", None, &host).as_deref(), Some("cursor:instance:cursor"));
        assert_eq!(continuation_key("cursor", Some(&work), &host).as_deref(), Some("cursor:instance:work"));
    }

    #[test]
    fn prepare_launch_materializes_a_codex_shadow_home() {
        let home = tempfile::tempdir().unwrap();
        let host = host(home.path());
        let shadow = account("personal", "", "~/.codex_personal");
        let path = PathBuf::from(prepare_launch("codex", &shadow, &host).unwrap().value("CODEX_HOME").unwrap());
        assert_eq!(path, home.path().join(".codex_personal"));
        assert_eq!(fs::read_link(path.join("sessions")).unwrap(), home.path().join(".codex/sessions"));
        assert!(prepare_launch("grok", &shadow, &host).is_err());
    }
}
