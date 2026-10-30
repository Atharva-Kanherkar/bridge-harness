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

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

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

/// Whether `harness` can carry accounts at all.
pub fn supports_accounts(harness: &str) -> bool {
    matches!(harness, "claude" | "codex")
}

/// The variable an account's home is handed to the provider through.
pub fn home_variable(harness: &str) -> Option<&'static str> {
    match harness {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        _ => None,
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
    match harness {
        "claude" => {
            if account.home_path.is_empty() {
                return Err(invalid(format!("account {id} needs a CLAUDE_CONFIG_DIR folder")));
            }
            if !account.shadow_home_path.is_empty() {
                return Err(invalid(format!(
                    "account {id}: Claude has no shared-home arrangement; remove shadowHomePath"
                )));
            }
        }
        _ => {
            if account.home_path.is_empty() && account.shadow_home_path.is_empty() {
                return Err(invalid(format!("account {id} needs a CODEX_HOME folder")));
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
        }
    }
}

/// The Claude config directory a chat runs in: the account's folder, else the
/// inherited `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_home(account: Option<&ProviderAccount>, host: &HostHomes) -> PathBuf {
    let configured = account.map(|account| account.home_path.as_str()).unwrap_or("");
    if !configured.is_empty() {
        return normalize(&expand_home_path(configured, &host.home));
    }
    // Inherited variables are not shell-expanded either, so a literal `~`
    // stays literal, as it would for Claude Code itself.
    host.claude_config_dir
        .as_deref()
        .map(normalize)
        .unwrap_or_else(|| host.home.join(".claude"))
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
    match harness {
        "claude" => Some(format!("claude:home:{}", claude_home(account, host).display())),
        "codex" => Some(codex_home_layout(account, host).continuation_key),
        _ => None,
    }
}

/// Make an account's home ready and return what its provider variable is set
/// to. Codex shadow homes are linked to their shared home first.
pub fn prepare_launch(
    harness: &str,
    account: &ProviderAccount,
    host: &HostHomes,
) -> Result<PathBuf, BridgeError> {
    match harness {
        "claude" => Ok(claude_home(Some(account), host)),
        "codex" => {
            let layout = codex_home_layout(Some(account), host);
            materialize_codex_shadow_home(&layout)?;
            layout
                .effective_home_path
                .ok_or_else(|| invalid(format!("account {} has no CODEX_HOME folder", account.id)))
        }
        other => Err(invalid(format!("{other} does not support extra accounts"))),
    }
}

/// Point a provider launch at an account's home. Only the provider's own
/// variable changes: Claude keeps `HOME`, since moving it would hide the
/// Keychain login (T3 `makeClaudeEnvironment`).
pub fn apply_account_home(command: &mut std::process::Command, harness: &str, home: Option<&Path>) {
    if let (Some(variable), Some(home)) = (home_variable(harness), home) {
        command.env(variable, home);
    }
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
    if shared == shadow {
        return Err(invalid(format!(
            "Codex shadow home {} must be different from the shared home {}",
            shadow.display(),
            shared.display()
        )));
    }
    let fs_error = |operation: &str, path: &Path, error: io::Error| {
        invalid(format!(
            "Codex shadow home: could not {operation} {}: {error}",
            path.display()
        ))
    };
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
            fs::remove_file(&path).map_err(|e| fs_error("remove", &path, e))?;
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
    let create = || std::os::unix::fs::symlink(&target, &link).map_err(io_error("link", &link));
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
            if fs::symlink_metadata(&link).is_ok_and(|meta| meta.is_dir()) {
                fs::remove_dir_all(&link).map_err(io_error("remove", &link))?;
            } else {
                fs::remove_file(&link).map_err(io_error("remove", &link))?;
            }
            create()
        }
        LinkState::Symlink(existing) => {
            let resolved = normalize(&link.parent().unwrap_or(shadow).join(existing));
            if resolved != target {
                fs::remove_file(&link).map_err(io_error("remove", &link))?;
                create()?;
            }
            Ok(())
        }
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
    match harness {
        "claude" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
}

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host(home: &Path) -> HostHomes {
        HostHomes { home: home.to_path_buf(), claude_config_dir: None, codex_home: None }
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
        let error = accounts("opencode", &json!({"accounts": [{"id": "a", "label": "A", "homePath": "~/.o"}]}));
        assert!(error.unwrap_err().to_string().contains("does not support extra accounts"));
        // An absent key is not an account list, for any harness.
        assert!(accounts("opencode", &json!({"visibleModels": []})).unwrap().is_empty());
        assert_eq!(normalize_choice("claude", Some("claude")), None);
        assert_eq!(normalize_choice("claude", Some("  ")), None);
        assert_eq!(normalize_choice("claude", None), None);
        assert_eq!(normalize_choice("claude", Some(" personal ")), Some("personal".into()));
        assert_eq!(home_variable("claude"), Some("CLAUDE_CONFIG_DIR"));
        assert_eq!(home_variable("codex"), Some("CODEX_HOME"));
        assert_eq!(home_variable("opencode"), None);
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
        assert_eq!(continuation_key("opencode", None, &host), None);
        assert_eq!(prepare_launch("claude", &personal, &host).unwrap(), PathBuf::from("/home/ada/.claude_personal"));
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
            apply_account_home(&mut command, harness, home);
            command
                .get_envs()
                .map(|(key, value)| (key.to_string_lossy().into_owned(), value.map(|v| v.to_string_lossy().into_owned())))
                .collect::<Vec<_>>()
        };
        let home = Path::new("/home/ada/.claude_personal");
        assert_eq!(envs("claude", Some(home)), vec![("CLAUDE_CONFIG_DIR".into(), Some("/home/ada/.claude_personal".into()))]);
        assert_eq!(envs("codex", Some(home)), vec![("CODEX_HOME".into(), Some("/home/ada/.claude_personal".into()))]);
        assert!(envs("claude", None).is_empty(), "no account inherits the user's own login");
        assert!(envs("opencode", Some(home)).is_empty());
    }

    #[test]
    fn prepare_launch_materializes_a_codex_shadow_home() {
        let home = tempfile::tempdir().unwrap();
        let host = host(home.path());
        let shadow = account("personal", "", "~/.codex_personal");
        let path = prepare_launch("codex", &shadow, &host).unwrap();
        assert_eq!(path, home.path().join(".codex_personal"));
        assert_eq!(fs::read_link(path.join("sessions")).unwrap(), home.path().join(".codex/sessions"));
        assert!(prepare_launch("opencode", &shadow, &host).is_err());
    }
}
