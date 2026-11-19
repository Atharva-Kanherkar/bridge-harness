//! The tool choice of a chat: the connectors and skills that its launches
//! leave out. Bridge stores the choice (kind `chat_tool_selection_v1`) and
//! for Codex one `config/read` fact (kind `codex_skills_config_v1`), never an
//! inventory. A choice only removes items, so nothing here can add an allow
//! rule, remove a deny or change a permission.
use crate::session_forest::{self, EntryKind, SessionForest};
use crate::{chat_defaults, provider_accounts, runtime::BridgeCore, BridgeError};
use bridge_protocol::messages::{
    ChatModelChoice, ChatToolItem, ChatToolKind, ChatToolSelection, ChatToolSelectionStatus,
    ChatToolState, ChatToolTokens, ClaudeSkillChoice, ConnectorDescriptor,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLockWriteGuard, TryLockError};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub const KIND: &str = "chat_tool_selection_v1";
/// Half of the 128 KiB cap of `chat_defaults::read`, so a stored row always
/// reads back.
pub const MAX_ROW_BYTES: usize = 64 * 1024;
pub const CODEX_APPS: &str = "codex_apps";
/// The one Codex value that turns off `codex_apps`, with each app inside it.
pub const CODEX_APPS_OFF: &str = "features.apps=false";
const MAX_ID_BYTES: usize = 128;
const MAX_SERVERS: usize = 128;
const MAX_SKILLS: usize = 384;
const BRIDGE: &str = crate::mcp_apps::tool::SERVER_NAME;

pub const CODEX_SERVER_SOURCE: &str = "codex.mcpServerStatus bytes_div4_v1";
pub const CODEX_SKILL_SOURCE: &str = "codex.skills/list bytes_div4_v1";

const BRIDGE_REASON: &str = "Bridge owns this server. It stays as attached today.";
const CLAUDE_SKILL_NAME_REASON: &str =
    "The Claude SDK cannot name this skill in a list. It loads only when the chat keeps every skill.";
const CLAUDE_PLUGIN_REASON: &str = "A plugin is not a unit. Choose its servers and skills instead.";
const CODEX_KEY_REASON: &str =
    "This server key has characters outside A-Z, a-z, 0-9, _ and -. Bridge cannot turn it off safely.";
const CODEX_APP_REASON: &str =
    "Codex has no verified switch for one app. The Apps switch turns all apps on or off.";
const CODEX_PLUGIN_REASON: &str = "Codex has no verified switch for a plugin.";
const CODEX_SKILLS_TABLE_REASON: &str =
    "The Codex config has a skills key. Bridge does not change it.";
const CODEX_SKILLS_UNKNOWN_REASON: &str =
    "Bridge can check the Codex config for a skills key only after a turn of this chat.";
pub const CODEX_SKILLS_UNCONFIRMED_REASON: &str =
    "Bridge cannot confirm the Codex skills config now. Open this list while the chat runs.";
const OPENCODE_SKILLS_REASON: &str = "OpenCode has no verified switch for one skill.";
const OPENCODE_MARKER_REASON: &str =
    "This server name has { or }. OpenCode can expand it to another server, so Bridge cannot turn it off safely.";

/// The stored row: the choice and its own revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolSelectionRow {
    pub revision: u32,
    pub harness: String,
    pub account: Option<String>,
    #[serde(default)]
    pub servers_off: Vec<String>,
    #[serde(default)]
    pub codex_apps_off: bool,
    #[serde(default)]
    pub claude_skills: Option<ClaudeSkillChoice>,
    #[serde(default)]
    pub codex_skills_off: Vec<String>,
}

impl ToolSelectionRow {
    fn new(revision: u32, choice: ChatToolSelection) -> Self {
        let ChatToolSelection { harness, account, servers_off, codex_apps_off, claude_skills, codex_skills_off } = choice;
        Self { revision, harness, account, servers_off, codex_apps_off, claude_skills, codex_skills_off }
    }

    pub fn choice(&self) -> ChatToolSelection {
        ChatToolSelection {
            harness: self.harness.clone(),
            account: self.account.clone(),
            servers_off: self.servers_off.clone(),
            codex_apps_off: self.codex_apps_off,
            claude_skills: self.claude_skills.clone(),
            codex_skills_off: self.codex_skills_off.clone(),
        }
    }
}

fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

fn unreadable() -> BridgeError {
    invalid("The saved tool choice of this chat is not valid. Choose its tools again")
}

fn is_bridge(id: &str) -> bool {
    id.trim().eq_ignore_ascii_case(BRIDGE)
}

/// The skill name rules of the Claude Agent SDK (`WXe` in sdk.mjs 0.3.322).
/// The SDK refuses a list that holds a name that fails them.
pub fn claude_sdk_accepts(name: &str) -> bool {
    // JavaScript `trim` also removes U+FEFF, and Rust `trim` keeps it.
    let trimmed = name.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}');
    !(trimmed.is_empty()
        || trimmed != name
        || name.contains(['(', ')', ','])
        || name.chars().any(char::is_control)
        || name == "*"
        || name.ends_with(":*")
        || name.ends_with(" *")
        || name.starts_with('/')
        || name.contains("\\\\")
        || name.ends_with('\\'))
}

/// A Codex config key that a `-c mcp_servers.<key>.enabled=false` value can
/// hold with no quotes.
pub fn codex_key_is_safe(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn check_id(id: &str, name: &str) -> Result<(), BridgeError> {
    if id.is_empty() || id.len() > MAX_ID_BYTES || id.chars().any(char::is_control) {
        return Err(invalid(format!(
            "A {name} name must have 1 to {MAX_ID_BYTES} bytes and no control character"
        )));
    }
    Ok(())
}

fn check_ids(ids: &[String], max: usize, name: &str) -> Result<(), BridgeError> {
    if ids.len() > max {
        return Err(invalid(format!("A tool choice can hold at most {max} {name} names")));
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        check_id(id, name)?;
        if !seen.insert(id.as_str()) {
            return Err(invalid(format!("The {name} name {id:?} shows more than once")));
        }
    }
    Ok(())
}

/// Check a choice. The result holds the account in its stored form: the
/// harness id means the own login of the user, the same as no account.
pub fn check(choice: &ChatToolSelection) -> Result<ChatToolSelection, BridgeError> {
    let harness = choice.harness.as_str();
    if !matches!(harness, "claude" | "codex" | "opencode") {
        return Err(invalid(format!("A tool choice cannot target the harness {harness:?}")));
    }
    let account = provider_accounts::normalize_choice(harness, choice.account.as_deref());
    if let Some(account) = &account {
        check_id(account, "account")?;
    }
    check_ids(&choice.servers_off, MAX_SERVERS, "server")?;
    if choice.servers_off.iter().any(|id| is_bridge(id)) {
        return Err(invalid(
            "The Bridge server stays as attached today. A tool choice cannot leave it out",
        ));
    }
    match harness {
        "claude" => {
            if choice.codex_apps_off || !choice.codex_skills_off.is_empty() {
                return Err(invalid("A Claude tool choice cannot hold Codex items"));
            }
            if let Some(skills) = &choice.claude_skills {
                check_ids(&skills.only, MAX_SKILLS, "Claude skill")?;
                if let Some(name) = skills.only.iter().find(|name| !claude_sdk_accepts(name)) {
                    return Err(invalid(format!("The Claude SDK refuses the skill name {name:?}")));
                }
            }
        }
        "codex" => {
            if choice.claude_skills.is_some() {
                return Err(invalid("A Codex tool choice cannot hold Claude skills"));
            }
            if let Some(key) = choice.servers_off.iter().find(|key| !codex_key_is_safe(key)) {
                return Err(invalid(format!(
                    "The Codex server key {key:?} is outside the safe set A-Z, a-z, 0-9, _ and -"
                )));
            }
            if choice.servers_off.iter().any(|key| key == CODEX_APPS) {
                return Err(invalid("Use the Apps switch to leave out codex_apps"));
            }
            check_ids(&choice.codex_skills_off, MAX_SKILLS, "Codex skill")?;
            if let Some(name) = choice.codex_skills_off.iter().find(|name| name.contains(['"', '\\'])) {
                return Err(invalid(format!(
                    "The Codex skill name {name:?} cannot go in a config value"
                )));
            }
        }
        _ => {
            if choice.claude_skills.is_some() || choice.codex_apps_off || !choice.codex_skills_off.is_empty() {
                return Err(invalid("An OpenCode tool choice can leave out servers only"));
            }
            if let Some(name) = choice.servers_off.iter().find(|name| has_substitution_marker(name)) {
                return Err(invalid(format!(
                    "The OpenCode server name {name:?} has {{ or }}. OpenCode reads it as a substitution marker"
                )));
            }
        }
    }
    Ok(ChatToolSelection { account, ..choice.clone() })
}

/// OpenCode expands `{env:NAME}` and `{file:path}` in its config text, so a
/// key with a brace can become the key of another server.
fn has_substitution_marker(name: &str) -> bool {
    name.contains(['{', '}'])
}

fn check_size(row: &ToolSelectionRow) -> Result<(), BridgeError> {
    let bytes = serde_json::to_string(row).map_err(|error| invalid(error.to_string()))?.len();
    if bytes > MAX_ROW_BYTES {
        return Err(invalid(format!(
            "The tool choice needs {bytes} bytes. The limit is {MAX_ROW_BYTES} bytes"
        )));
    }
    Ok(())
}

/// The stored choice of a chat. A row that fails its checks is an error, so
/// no launch runs on a choice that Bridge cannot read.
pub fn read(db: &Connection, session_id: &str) -> Result<Option<ToolSelectionRow>, BridgeError> {
    let row = chat_defaults::read::<ToolSelectionRow>(db, KIND, session_id).map_err(|error| match error {
        BridgeError::Invalid(_) => unreadable(),
        other => other,
    })?;
    let Some(row) = row else { return Ok(None) };
    check(&row.choice()).map_err(|_| unreadable())?;
    Ok(Some(row))
}

/// The revision to compare with. A row that does not read counts as 0, so a
/// save can replace it.
fn stored_revision(db: &Connection, session_id: &str) -> Result<u32, BridgeError> {
    match chat_defaults::read::<Value>(db, KIND, session_id) {
        Ok(row) => Ok(row
            .and_then(|row| row.get("revision").and_then(Value::as_u64))
            .and_then(|revision| u32::try_from(revision).ok())
            .unwrap_or(0)),
        Err(BridgeError::Invalid(_)) => Ok(0),
        Err(other) => Err(other),
    }
}

/// Check a choice against the stored revision (0 when no row exists) and
/// write it as `revision + 1`. The caller owns the connection or the open
/// transaction, so the write commits with the rest of its work.
pub fn write_checked(
    db: &Connection,
    session_id: &str,
    expected_revision: u32,
    choice: &ChatToolSelection,
) -> Result<ToolSelectionRow, BridgeError> {
    let checked = check(choice)?;
    let stored = stored_revision(db, session_id)?;
    if stored != expected_revision {
        return Err(invalid("The tool choice changed in another window. Reload it and try again"));
    }
    let revision = stored
        .checked_add(1)
        .ok_or_else(|| invalid("The tool choice has no revision left"))?;
    let row = ToolSelectionRow::new(revision, checked);
    check_size(&row)?;
    chat_defaults::write(db, KIND, session_id, &row)?;
    Ok(row)
}

/// Save the first choice of a new chat inside its create transaction. A
/// choice for another harness or account fails the create, so nothing
/// commits.
pub fn write_first(
    db: &Connection,
    session_id: &str,
    model: &ChatModelChoice,
    choice: &ChatToolSelection,
) -> Result<ToolSelectionRow, BridgeError> {
    let checked = check(choice)?;
    let account = provider_accounts::normalize_choice(&model.harness, model.account.as_deref());
    if checked.harness != model.harness || checked.account != account {
        return Err(invalid(format!(
            "The tool choice was made for {}, but the chat starts on {}. Review its tools and try again",
            describe(&checked.harness, checked.account.as_deref()),
            describe(&model.harness, account.as_deref()),
        )));
    }
    write_checked(db, session_id, 0, &checked)
}

// --- save for a chat that exists --------------------------------------------------

/// The statuses in which a chat takes a new choice. `failed` is in the list,
/// so a person can roll a choice back after a failed launch.
const SAVE_STATUSES: &[&str] = &["ready", "idle", "stopped", "failed"];
const LEASE_RETRY: Duration = Duration::from_millis(50);
const LEASE_WAIT: Duration = Duration::from_secs(2);

/// The refusal while a send holds the input lease.
pub const SEND_BUSY: &str = "A message is on its way. Try again when it lands.";
const LIFECYCLE_BUSY: &str = "Another change to this chat runs now. Try again when it is done.";
const TURN_BUSY: &str = "A turn of this chat runs now. Try again when it ends.";
const CHECKPOINT_BUSY: &str = "This chat makes a checkpoint now. Try again when it is done.";
const APPROVAL_BUSY: &str = "This chat waits for your answer. Answer it, then try again.";
const START_BUSY: &str = "This chat starts now. Try again when it is ready.";
const STATUS_BUSY: &str = "This chat is busy. Try again when it is idle.";
const QUEUED_BUSY: &str = "A queued message waits for this chat. Try again after it goes out.";
const WORKER_BUSY: &str = "A worker result waits for this chat. Try again after it arrives.";
const SUBAGENT_BUSY: &str = "A subagent of this chat runs or waits for an answer. Try again when it is done.";
const TASK_BUSY: &str = "A background task of this chat runs. Try again when it ends.";

/// The result of [`save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The new row. Its revision is one more than the expected revision.
    Saved(ToolSelectionRow),
    /// The chat is busy, and nothing changed. The text tells the person why.
    Busy(&'static str),
}

/// The global input lease in write mode. A send holds the read lease from
/// its route to its provider write, so a save never lands between the two.
/// `None` when a send still holds it after the wait.
fn write_lease(core: &BridgeCore) -> Option<RwLockWriteGuard<'_, ()>> {
    let start = Instant::now();
    loop {
        match core.input_activity.try_write() {
            Ok(lease) => return Some(lease),
            // The lease guards no data, so a panic under it left nothing half
            // written.
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        let waited = start.elapsed();
        if waited >= LEASE_WAIT {
            return None;
        }
        std::thread::sleep(LEASE_RETRY.min(LEASE_WAIT - waited));
    }
}

/// The text for a chat that the shared idle check does not pass.
fn busy_reason(activity: &crate::runtime_budget::Activity) -> &'static str {
    if !activity.status_fits {
        return match activity.status.as_str() {
            "working" => TURN_BUSY,
            "checkpointing" => CHECKPOINT_BUSY,
            "waiting" => APPROVAL_BUSY,
            "starting" => START_BUSY,
            _ => STATUS_BUSY,
        };
    }
    if activity.active_turn {
        TURN_BUSY
    } else if activity.queued_input {
        QUEUED_BUSY
    } else if activity.worker_result {
        WORKER_BUSY
    } else if activity.subagent {
        SUBAGENT_BUSY
    } else if activity.background_task {
        TASK_BUSY
    } else {
        STATUS_BUSY
    }
}

/// The refusal of a save for a chat that is busy now, or `None` for an idle
/// chat: the shared idle check under the database lock, then the turn of a
/// warm runtime.
fn busy_now(core: &BridgeCore, db: &Connection, session_id: &str) -> Result<Option<&'static str>, BridgeError> {
    let activity = crate::runtime_budget::activity(db, session_id, SAVE_STATUSES)?
        .ok_or_else(|| invalid("This chat does not exist"))?;
    if !activity.chat || !chat_defaults::is_top_level_chat(db, session_id)? {
        return Err(invalid("Only a top-level chat can have a tool choice"));
    }
    if !activity.idle() {
        return Ok(Some(busy_reason(&activity)));
    }
    let turn_runs = core
        .adapters
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session_id)
        .is_some_and(|runtime| runtime.current_turn().lock().unwrap_or_else(PoisonError::into_inner).is_some());
    Ok(turn_runs.then_some(TURN_BUSY))
}

/// Save a new choice for a chat that exists. The save takes the guard of the
/// idle release in the same order: the input lease, the lifecycle claim of
/// the session, then [`busy_now`]. A busy chat gets a refusal, and nothing
/// changes. The save never stops the process: the next turn applies the new
/// revision.
pub fn save(
    core: &BridgeCore,
    session_id: &str,
    expected_revision: u32,
    selection: &ChatToolSelection,
) -> Result<SaveOutcome, BridgeError> {
    check(selection)?;
    let Some(lease) = write_lease(core) else {
        return Ok(SaveOutcome::Busy(SEND_BUSY));
    };
    // A first check without the claim refuses a busy chat early, so a cancel
    // of its turn never fails because a save waits for the database.
    if let Some(reason) = busy_now(core, &*core.try_db()?, session_id)? {
        return Ok(SaveOutcome::Busy(reason));
    }
    let Ok(claim) = core.claim_session_lifecycle(session_id, "tool selection") else {
        return Ok(SaveOutcome::Busy(LIFECYCLE_BUSY));
    };
    let db = core.try_db()?;
    if let Some(reason) = busy_now(core, &db, session_id)? {
        return Ok(SaveOutcome::Busy(reason));
    }
    let transaction = db.unchecked_transaction()?;
    let row = write_checked(&transaction, session_id, expected_revision, selection)?;
    transaction.commit()?;
    // The claim goes before the lease, so a send that waits on the lease
    // finds the session free when it starts the chat.
    drop(db);
    drop(claim);
    drop(lease);
    Ok(SaveOutcome::Saved(row))
}

fn describe(harness: &str, account: Option<&str>) -> String {
    let label = match harness {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        other => other,
    };
    match account {
        Some(account) => format!("{label} (account {account})"),
        None => format!("{label} (own login)"),
    }
}

/// Whether a stored choice applies to a launch on this harness and account.
/// A choice that does not apply stays stored, with a label that says so.
pub fn status(row: &ToolSelectionRow, harness: &str, account: Option<&str>) -> ChatToolSelectionStatus {
    let account = provider_accounts::normalize_choice(harness, account);
    let applies = row.harness == harness && row.account == account;
    ChatToolSelectionStatus {
        revision: row.revision,
        harness: row.harness.clone(),
        account: row.account.clone(),
        applies,
        not_applied_reason: (!applies).then(|| {
            format!("Chosen for {}, not applied", describe(&row.harness, row.account.as_deref()))
        }),
    }
}

/// What one launch leaves out. It travels beside the start or resume
/// request. The lists are sorted, so equal sets compare equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLaunch {
    pub revision: u32,
    pub harness: String,
    /// Never holds the Bridge server.
    pub servers_off: Vec<String>,
    pub codex_apps_off: bool,
    /// `None` keeps every Claude skill. `Some` keeps only the listed skills.
    pub claude_skills: Option<Vec<String>>,
    pub codex_skills_off: Vec<String>,
    /// Stable for equal effective sets. The revision is not part of it.
    pub effective_hash: String,
    /// The hash of the stored choice. It differs from `effective_hash` when
    /// the launch cannot apply a part of the choice.
    pub stored_hash: String,
}

impl ToolLaunch {
    pub fn from_row(row: &ToolSelectionRow) -> Self {
        let servers_off = sorted(row.servers_off.iter().filter(|id| !is_bridge(id)));
        let claude_skills = row.claude_skills.as_ref().map(|skills| sorted(skills.only.iter()));
        let codex_skills_off = sorted(row.codex_skills_off.iter());
        let effective_hash = hash_of(
            &row.harness,
            &servers_off,
            row.codex_apps_off,
            claude_skills.as_deref(),
            &codex_skills_off,
        );
        Self {
            revision: row.revision,
            harness: row.harness.clone(),
            servers_off,
            codex_apps_off: row.codex_apps_off,
            claude_skills,
            codex_skills_off,
            stored_hash: effective_hash.clone(),
            effective_hash,
        }
    }

    /// Keep the Codex skills choice only when a stored `config/read` fact
    /// confirms that the config of the launch has no `skills` key of the
    /// user. A `skills.config` value replaces that key, so in each other case
    /// the launch leaves the skills as they are. The effective hash follows
    /// the set that the launch applies.
    pub fn with_codex_skills_known(self, no_user_skills_key: bool) -> Self {
        if self.harness != "codex" || self.codex_skills_off.is_empty() || no_user_skills_key {
            return self;
        }
        let codex_skills_off = Vec::new();
        let effective_hash =
            hash_of(&self.harness, &self.servers_off, self.codex_apps_off, self.claude_skills.as_deref(), &codex_skills_off);
        Self { codex_skills_off, effective_hash, ..self }
    }

    /// Keep the OpenCode servers choice only when the inherited config can
    /// hold it. Otherwise the adapter keeps the config of today, so the launch
    /// leaves no server out and claims none.
    pub fn with_opencode_config(self, existing: Option<&str>) -> Self {
        if self.harness != "opencode" || opencode_not_applied_reason(existing, &self.servers_off).is_none() {
            return self;
        }
        let servers_off = Vec::new();
        let effective_hash =
            hash_of(&self.harness, &servers_off, self.codex_apps_off, self.claude_skills.as_deref(), &self.codex_skills_off);
        Self { servers_off, effective_hash, ..self }
    }
}

fn sorted<'a>(ids: impl Iterator<Item = &'a String>) -> Vec<String> {
    ids.cloned().collect::<BTreeSet<_>>().into_iter().collect()
}

/// The hash of an effective set. Each list must be sorted.
fn hash_of(
    harness: &str,
    servers_off: &[String],
    codex_apps_off: bool,
    claude_skills: Option<&[String]>,
    codex_skills_off: &[String],
) -> String {
    let canonical = serde_json::json!({
        "v": 1,
        "harness": harness,
        "serversOff": servers_off,
        "codexAppsOff": codex_apps_off,
        "claudeSkills": claude_skills,
        "codexSkillsOff": codex_skills_off,
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

/// The hash of a launch that leaves nothing out, the same as a chat with no
/// stored choice.
pub fn all_on_hash(harness: &str) -> String {
    hash_of(harness, &[], false, None, &[])
}

/// The target of one launch, as `live_turn` knows it.
#[derive(Debug, Clone, Copy)]
pub struct LaunchTarget<'a> {
    /// The agent harness of the chat, as `sessions.harness` records it.
    pub harness: &'a str,
    /// A top-level chat that a person reads.
    pub interactive: bool,
    /// A workflow run, or a launch under a briefing policy.
    pub workflow: bool,
    /// The folder of the launch.
    pub cwd: &'a str,
    /// The `OPENCODE_CONFIG_CONTENT` that the launch inherits.
    pub opencode_config: Option<&'a str>,
}

/// The tool launch of a chat: `None` for a chat with no stored choice, and
/// `None` unless the session is an interactive top-level chat outside a
/// workflow and its stored choice applies. Workers, briefings and workflows
/// never read the row. A Codex skills choice applies only when a stored
/// `config/read` fact confirms that the config of this account and folder
/// has no `skills` key of the user. An OpenCode servers choice applies only
/// when the inherited config can hold it.
pub fn launch_for(
    db: &Connection,
    session_id: &str,
    target: &LaunchTarget<'_>,
) -> Result<Option<ToolLaunch>, BridgeError> {
    if !target.interactive || target.workflow || !chat_defaults::is_top_level_chat(db, session_id)? {
        return Ok(None);
    }
    let Some(row) = read(db, session_id)? else { return Ok(None) };
    let account: Option<String> = db.query_row(
        "SELECT account FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    if !status(&row, target.harness, account.as_deref()).applies {
        return Ok(None);
    }
    let launch = ToolLaunch::from_row(&row);
    // The file reads run only for a choice that has a skills part.
    let no_user_skills_key = launch.harness == "codex" && !launch.codex_skills_off.is_empty() && {
        let place = CodexPlace::resolve(db, account.as_deref(), target.cwd);
        codex_skills_key(db, session_id, place.as_ref()) == CodexSkillsKey::Known(false)
    };
    Ok(Some(launch.with_codex_skills_known(no_user_skills_key).with_opencode_config(target.opencode_config)))
}

// --- the Codex skills config ------------------------------------------------------

/// The kind of the stored `config/read` fact of a Codex chat.
pub const CODEX_CONFIG_KIND: &str = "codex_skills_config_v1";
const MAX_LAYER_FILES: usize = 64;
const MAX_PATH_BYTES: usize = 4096;

/// The account and the folder of a Codex chat, and the config files that
/// Codex reads for them: `config.toml` in the Codex home of the account and
/// each `.codex/config.toml` from the folder up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPlace {
    account: Option<String>,
    cwd: String,
    files: BTreeSet<PathBuf>,
}

impl CodexPlace {
    /// `None` when the account is no longer in Settings.
    pub fn resolve(db: &Connection, account: Option<&str>, cwd: &str) -> Option<Self> {
        let account = provider_accounts::normalize_choice("codex", account);
        let record = match &account {
            Some(id) => Some(crate::agent_config::account(db, "codex", id).ok()?),
            None => None,
        };
        let layout = provider_accounts::codex_home_layout(record.as_ref(), &provider_accounts::HostHomes::from_env());
        let homes = std::iter::once(layout.shared_home_path).chain(layout.effective_home_path);
        let mut files: BTreeSet<PathBuf> = homes.map(|home| home.join("config.toml")).collect();
        files.extend(Path::new(cwd).ancestors().map(|folder| folder.join(".codex").join("config.toml")));
        Some(Self { account, cwd: cwd.to_owned(), files })
    }

    /// The fingerprint of the files of the place.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.files)
    }

    fn with(&self, layer_files: &[String]) -> BTreeSet<PathBuf> {
        let mut files = self.files.clone();
        files.extend(layer_files.iter().map(PathBuf::from));
        files
    }
}

/// The length and the modification time of each file, or `absent`.
fn fingerprint(files: &BTreeSet<PathBuf>) -> String {
    let mut digest = Sha256::new();
    for file in files {
        let state = match std::fs::metadata(file) {
            Ok(metadata) => {
                let modified = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok());
                format!("{}:{}", metadata.len(), modified.map_or(0, |time| time.as_nanos()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_owned(),
            Err(error) => format!("unreadable:{:?}", error.kind()),
        };
        digest.update(file.as_os_str().as_encoded_bytes());
        digest.update([0]);
        digest.update(state.as_bytes());
        digest.update([b'\n']);
    }
    format!("{:x}", digest.finalize())
}

/// One `config/read` of a Codex chat, bound to what Codex read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexConfigFact {
    account: Option<String>,
    cwd: String,
    user_skills_key: bool,
    /// The files of the layers of the reply.
    files: Vec<String>,
    /// The fingerprint of the files of the place and of the layers.
    fingerprint: String,
}

/// What Bridge knows about a `skills` key of the user in the Codex config of
/// a chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexSkillsKey {
    /// No `config/read` of the chat is stored.
    Unknown,
    /// The stored read is for another account or folder, or a config file
    /// changed after it.
    Unconfirmed,
    /// The stored read still holds: `true` when the config has the key.
    Known(bool),
}

/// The stored fact of a chat, checked against a place. A fresh fingerprint
/// must equal the stored one.
pub fn codex_skills_key(db: &Connection, session_id: &str, place: Option<&CodexPlace>) -> CodexSkillsKey {
    let fact = match chat_defaults::read::<CodexConfigFact>(db, CODEX_CONFIG_KIND, session_id) {
        Ok(Some(fact)) => fact,
        Ok(None) => return CodexSkillsKey::Unknown,
        Err(_) => return CodexSkillsKey::Unconfirmed,
    };
    let Some(place) = place.filter(|place| place.account == fact.account && place.cwd == fact.cwd) else {
        return CodexSkillsKey::Unconfirmed;
    };
    if fingerprint(&place.with(&fact.files)) != fact.fingerprint {
        return CodexSkillsKey::Unconfirmed;
    }
    CodexSkillsKey::Known(fact.user_skills_key)
}

/// Store the fact of a translated `config` reply (`userSkillsKey` and
/// `files` of `apps::codex`) for a place. `before` is the fingerprint of
/// the place before the read. A reply without the flag removes the stored
/// fact. The result is true when the fact is stored.
pub fn remember_codex_config(
    db: &Connection,
    session_id: &str,
    place: &CodexPlace,
    before: &str,
    reply: &Value,
) -> Result<bool, BridgeError> {
    let Some(user_skills_key) = reply.get("userSkillsKey").and_then(Value::as_bool) else {
        db.execute(
            "DELETE FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![CODEX_CONFIG_KIND, session_id],
        )?;
        return Ok(false);
    };
    // A file that changed during the read can hold what the reply missed.
    if place.fingerprint() != before {
        return Ok(false);
    }
    let files: Vec<String> = reply_list(Some(reply), "files")
        .iter()
        .filter_map(Value::as_str)
        .filter(|file| file.len() <= MAX_PATH_BYTES)
        .take(MAX_LAYER_FILES)
        .map(str::to_owned)
        .collect();
    let fact = CodexConfigFact {
        account: place.account.clone(),
        cwd: place.cwd.clone(),
        user_skills_key,
        fingerprint: fingerprint(&place.with(&files)),
        files,
    };
    chat_defaults::write(db, CODEX_CONFIG_KIND, session_id, &fact)?;
    Ok(true)
}

// --- the choice that a warm runtime applies ---------------------------------------

/// The entry kind that records a changed effective set.
pub const CHANGED_KIND: &str = "session.tool_selection_changed";

/// What the warm runtime of a chat launched with. Only `start_chat` writes an
/// entry. `start_session` and the reader exit of the same launch remove one.
/// The frame path only settles `pending` and `confirmed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTools {
    /// The stored revision at the launch: 0 for a chat with no row.
    pub revision: u32,
    pub effective_hash: String,
    /// False while a Claude event waits for its proof.
    pub confirmed: bool,
    pub pending: Option<PendingEvent>,
    /// True when the launch applied less than the stored choice.
    pub partial: bool,
    /// The `started_at` of the launch.
    pub started_at: String,
}

/// A Claude event that waits for the first good turn on the same session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEvent {
    pub payload: Value,
    pub provider_session_id: String,
    /// The `started_at` of the launch. A frame of another launch cannot
    /// settle the event.
    pub started_at: String,
    /// The session id of the last init frame of the launch.
    pub init_session_id: Option<String>,
}

impl PendingEvent {
    /// `launch` is the `started_at` and the provider session id of the
    /// launch of a frame. `None` is a frame with no launch to check.
    fn is_of(&self, launch: Option<(&str, &str)>) -> bool {
        launch.is_none_or(|(started_at, provider_session_id)| {
            self.started_at == started_at && self.provider_session_id == provider_session_id
        })
    }
}

/// The effective set of one launch, read before the launch starts.
#[derive(Debug, Clone)]
pub struct LaunchRecord {
    revision: u32,
    harness: String,
    tools: Option<ToolLaunch>,
    effective_hash: String,
    changed: bool,
    /// The stored choice differs from the last event too, so the person
    /// changed the set. A launch that only drops a part that it cannot apply
    /// is not a choice of the person.
    chosen: bool,
    partial: bool,
    prefix_rewritten: bool,
}

impl LaunchRecord {
    /// "Changed" compares the effective hash with the last event of the chat
    /// for the same harness, or with the "all on" hash when there is none.
    /// `prefix_rewritten` is false only for the first launch of a chat.
    pub fn read(
        db: &Connection,
        session_id: &str,
        harness: &str,
        tools: Option<&ToolLaunch>,
        prefix_rewritten: bool,
    ) -> Result<Self, BridgeError> {
        let revision = match tools {
            Some(tools) => tools.revision,
            None => stored_revision(db, session_id)?,
        };
        let effective_hash = tools.map_or_else(|| all_on_hash(harness), |tools| tools.effective_hash.clone());
        let last: Option<(Option<String>, Option<String>)> = db
            .query_row(
                "SELECT json_extract(payload,'$.harness'),json_extract(payload,'$.effectiveHash') FROM session_entries WHERE session_id=?1 AND kind=?2 ORDER BY sequence DESC LIMIT 1",
                params![session_id, CHANGED_KIND],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let last_hash = match last {
            Some((Some(last_harness), Some(hash))) if last_harness == harness => hash,
            _ => all_on_hash(harness),
        };
        let stored_hash = tools.map_or(effective_hash.as_str(), |tools| tools.stored_hash.as_str());
        Ok(Self {
            revision,
            harness: harness.to_owned(),
            tools: tools.cloned(),
            changed: effective_hash != last_hash,
            chosen: stored_hash != last_hash,
            partial: stored_hash != effective_hash,
            effective_hash,
            prefix_rewritten,
        })
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    /// A failed native resume with a changed set keeps the thread on each
    /// harness: no fork and no fresh start. A set that changed only because
    /// the launch drops a part keeps the fallback, so a person can always
    /// leave a failed chat.
    pub fn keeps_the_thread_on_failure(&self) -> bool {
        self.changed && self.chosen
    }

    /// Claude gives no proof at launch. The proof is the first good turn.
    pub fn waits_for_a_turn(&self) -> bool {
        self.changed && self.harness == "claude"
    }

    /// The payload of the event, or `None` for an unchanged set.
    pub fn event(&self, provider_session_id: &str) -> Option<Value> {
        if !self.changed {
            return None;
        }
        let tools = self.tools.as_ref();
        let none: &[String] = &[];
        let claude_skills = match tools.and_then(|tools| tools.claude_skills.as_ref()) {
            Some(skills) => serde_json::json!(skills),
            None => serde_json::json!("all"),
        };
        Some(serde_json::json!({
            "revision": self.revision,
            "harness": self.harness,
            "effectiveHash": self.effective_hash,
            "serversOff": tools.map_or(none, |tools| tools.servers_off.as_slice()),
            "codexAppsOff": tools.is_some_and(|tools| tools.codex_apps_off),
            "codexSkillsOff": tools.map_or(none, |tools| tools.codex_skills_off.as_slice()),
            "claudeSkills": claude_skills,
            "providerSessionId": provider_session_id,
            "prefixRewritten": self.prefix_rewritten,
        }))
    }

    /// Write the event of a changed set in the transaction of the `sessions`
    /// update of a launch that succeeded. Claude waits for its first turn.
    pub fn write_launch_event(
        &self,
        transaction: &Transaction<'_>,
        session_id: &str,
        provider_session_id: &str,
    ) -> Result<(), BridgeError> {
        if self.waits_for_a_turn() {
            return Ok(());
        }
        let Some(payload) = self.event(provider_session_id) else { return Ok(()) };
        session_forest::append_in_transaction(transaction, session_id, EntryKind::ToolSelectionChanged, payload)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        Ok(())
    }

    /// The entry of a launch that succeeded at `started_at`.
    pub fn applied(&self, provider_session_id: &str, started_at: &str) -> AppliedTools {
        let pending = self
            .waits_for_a_turn()
            .then(|| self.event(provider_session_id))
            .flatten()
            .map(|payload| PendingEvent {
                payload,
                provider_session_id: provider_session_id.to_owned(),
                started_at: started_at.to_owned(),
                init_session_id: None,
            });
        AppliedTools {
            revision: self.revision,
            effective_hash: self.effective_hash.clone(),
            confirmed: pending.is_none(),
            pending,
            partial: self.partial,
            started_at: started_at.to_owned(),
        }
    }
}

/// Keep the entry of a launch that succeeded.
pub fn record_applied(core: &BridgeCore, session_id: &str, applied: AppliedTools) {
    core.applied_tools
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session_id.to_owned(), applied);
}

/// Remove the entry of a chat whose new runtime launched with no tool choice.
pub fn forget_applied(core: &BridgeCore, session_id: &str) {
    core.applied_tools.lock().unwrap_or_else(PoisonError::into_inner).remove(session_id);
}

/// Remove the entry of the launch at `started_at` when its runtime goes away.
/// The entry of a newer launch stays.
pub fn forget_applied_launch(core: &BridgeCore, session_id: &str, started_at: &str) {
    let mut applied = core.applied_tools.lock().unwrap_or_else(PoisonError::into_inner);
    if applied.get(session_id).is_some_and(|entry| entry.started_at == started_at) {
        applied.remove(session_id);
    }
}

/// Remove the entry of a warm runtime that applied less than the stored
/// choice, so the next send relaunches it with the whole choice.
pub fn forget_partial(core: &BridgeCore, session_id: &str) {
    let mut applied = core.applied_tools.lock().unwrap_or_else(PoisonError::into_inner);
    if applied.get(session_id).is_some_and(|entry| entry.partial) {
        applied.remove(session_id);
    }
}

/// The revision that the warm runtime of a chat applies: 0 with no entry.
pub fn applied_revision(core: &BridgeCore, session_id: &str) -> u32 {
    core.applied_tools
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(session_id)
        .map_or(0, |applied| applied.revision)
}

/// True when the warm runtime of a chat applies another revision than the
/// stored one. It reads one row and one map entry. A failed read keeps the
/// warm runtime.
pub fn stale(core: &BridgeCore, session_id: &str) -> bool {
    let Ok(stored) = core.try_db().and_then(|db| stored_revision(&db, session_id)) else {
        return false;
    };
    applied_revision(core, session_id) != stored
}

/// Keep the session id of an init frame for the pending event of its launch.
/// The frame path calls it, so the end of the turn reads no history.
pub fn note_init(core: &BridgeCore, session_id: &str, launch: Option<(&str, &str)>, init_session_id: &str) {
    let mut applied = core.applied_tools.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(pending) = applied.get_mut(session_id).and_then(|entry| entry.pending.as_mut()) else { return };
    if pending.is_of(launch) {
        pending.init_session_id = Some(init_session_id.to_owned());
    }
}

/// Settle the Claude event of the last launch at the end of a turn of that
/// launch. A failed turn keeps the event in wait, and so does a turn of
/// another launch. A good turn writes the event when the last init frame of
/// the launch has the same session id, and drops it when not.
pub fn settle_after_turn(core: &BridgeCore, session_id: &str, turn_failed: bool, launch: Option<(&str, &str)>) {
    if turn_failed {
        return;
    }
    // The map lock comes before the database lock here and nowhere after it.
    let mut applied = core.applied_tools.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(entry) = applied.get_mut(session_id) else { return };
    if !entry.pending.as_ref().is_some_and(|pending| pending.is_of(launch)) {
        return;
    }
    let Some(pending) = entry.pending.take() else { return };
    if pending.init_session_id.as_deref() != Some(pending.provider_session_id.as_str()) {
        return;
    }
    let written = core.try_db().and_then(|db| {
        SessionForest::new(&db)
            .append(session_id, EntryKind::ToolSelectionChanged, pending.payload)
            .map_err(|error| BridgeError::Invalid(error.to_string()))
    });
    match written {
        Ok(_) => entry.confirmed = true,
        Err(error) => crate::diagnostics::record(&format!(
            "tool_selection: the change event of session {session_id} was not written: {error}"
        )),
    }
}

/// Name the server key of a Codex launch that failed because the key left
/// the config. Codex refuses `-c mcp_servers.<key>.enabled=false` for such a
/// key with "invalid transport in mcp_servers.<key>" on stderr.
pub fn explain_launch_error(tools: Option<&ToolLaunch>, error: BridgeError) -> BridgeError {
    let Some(tools) = tools.filter(|tools| tools.harness == "codex") else { return error };
    let text = error.to_string();
    if !text.contains("invalid transport") {
        return error;
    }
    match tools.servers_off.iter().find(|key| names_codex_key(&text, key)) {
        Some(key) => invalid(format!("The saved tool choice names a Codex server that is no longer configured: {key}")),
        None => error,
    }
}

fn names_codex_key(text: &str, key: &str) -> bool {
    let needle = format!("mcp_servers.{key}");
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_' || next == '-'))
    })
}

// --- launch translations ----------------------------------------------------------

/// The `toolSelection` value of the Claude sidecar config: the servers to
/// deny and the skills to keep. The skill list is absent when the chat keeps
/// every skill. The deny list never names the Bridge server. The sidecar
/// adds the names to its deny list and never removes an entry.
pub fn claude_sidecar_selection(tools: &ToolLaunch) -> Option<Value> {
    if tools.harness != "claude" {
        return None;
    }
    let denied: Vec<&str> = tools.servers_off.iter().map(String::as_str).filter(|id| !is_bridge(id)).collect();
    let mut selection = serde_json::Map::new();
    selection.insert("deniedMcpServers".into(), Value::from(denied));
    if let Some(skills) = &tools.claude_skills {
        selection.insert("skills".into(), Value::from(skills.clone()));
    }
    Some(Value::Object(selection))
}

/// A TOML basic string. A quote, a backslash and each control character
/// get an escape, so a name cannot close the string or add a key.
fn toml_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for ch in text.chars() {
        match ch {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            ch if ch.is_control() => quoted.push_str(&format!("\\u{:04X}", u32::from(ch))),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

/// The root `-c` values of a Codex launch, in three forms only: one
/// `mcp_servers.<key>.enabled=false` for each off server, `features.apps=false`,
/// and one `skills.config` list of `enabled=false` entries. No value turns a
/// thing on or names the Bridge server. A key outside the safe set gets no
/// value, because the checks refuse it and a quote must never reach the key.
pub fn codex_overrides(tools: &ToolLaunch) -> Vec<String> {
    if tools.harness != "codex" {
        return Vec::new();
    }
    let mut values: Vec<String> = tools
        .servers_off
        .iter()
        .filter(|key| !is_bridge(key) && key.as_str() != CODEX_APPS && codex_key_is_safe(key))
        .map(|key| format!("mcp_servers.{key}.enabled=false"))
        .collect();
    if tools.codex_apps_off {
        values.push(CODEX_APPS_OFF.to_owned());
    }
    if !tools.codex_skills_off.is_empty() {
        let entries: Vec<String> =
            tools.codex_skills_off.iter().map(|name| format!("{{name={},enabled=false}}", toml_string(name))).collect();
        values.push(format!("skills.config=[{}]", entries.join(",")));
    }
    values
}

/// The picker reason for an OpenCode choice that a launch cannot apply.
pub const OPENCODE_CONFIG_REASON: &str = "Not applied: the inherited OpenCode config is not valid JSON";

/// The result of [`merge_opencode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCodeMerge {
    /// The config content with each left-out server turned off.
    Applied(String),
    /// The inherited content is not a JSON object, so the choice is not
    /// applied and the launch keeps the content of today.
    NotApplied,
}

/// Set `mcp.<key>.enabled=false` for each server in `off` and keep each other
/// key. No content counts as the empty object. Content that does not parse
/// is never replaced, because it can hold a setting of the user.
pub fn merge_opencode(existing: Option<&str>, off: &[String]) -> OpenCodeMerge {
    let mut config = match existing.map(serde_json::from_str::<Value>) {
        None => serde_json::Map::new(),
        Some(Ok(Value::Object(config))) => config,
        Some(_) => return OpenCodeMerge::NotApplied,
    };
    let off: Vec<&String> = off.iter().filter(|key| !is_bridge(key) && !has_substitution_marker(key)).collect();
    if !off.is_empty() {
        let Some(servers) = config.entry("mcp").or_insert_with(|| Value::Object(serde_json::Map::new())).as_object_mut() else {
            return OpenCodeMerge::NotApplied;
        };
        for key in off {
            let entry = servers.entry(key.as_str()).or_insert_with(|| Value::Object(serde_json::Map::new()));
            let Some(entry) = entry.as_object_mut() else { return OpenCodeMerge::NotApplied };
            entry.insert("enabled".into(), Value::Bool(false));
        }
    }
    OpenCodeMerge::Applied(Value::Object(config).to_string())
}

/// The reason that a launch does not apply these off servers with this
/// inherited `OPENCODE_CONFIG_CONTENT`, or `None` when it applies them.
pub fn opencode_not_applied_reason(existing: Option<&str>, off: &[String]) -> Option<&'static str> {
    (merge_opencode(existing, off) == OpenCodeMerge::NotApplied).then_some(OPENCODE_CONFIG_REASON)
}

// --- units ------------------------------------------------------------------------

/// Token figures of one reading, by item id, with the name of their source.
#[derive(Debug, Clone, Default)]
pub struct TokenFigures {
    pub source: String,
    pub at: Option<String>,
    pub servers: BTreeMap<String, u64>,
    pub skills: BTreeMap<String, u64>,
}

fn figure(figures: Option<&TokenFigures>, skill: bool, id: &str) -> Option<ChatToolTokens> {
    let figures = figures?;
    let values = if skill { &figures.skills } else { &figures.servers };
    let value = values.get(id)?;
    Some(ChatToolTokens { value: *value, source: figures.source.clone(), at: figures.at.clone() })
}

fn bytes_div4(bytes: usize) -> u64 {
    u64::try_from(bytes / 4).unwrap_or(u64::MAX)
}

/// The `bytes_div4_v1` figure of one server of a `mcpServerStatus/list`
/// reply: a quarter of the bytes of its `tools` map. `None` for a server
/// with no tools, such as a server that is off.
pub fn codex_tools_tokens(server: &Value) -> Option<u64> {
    let tools = server.get("tools")?.as_object().filter(|tools| !tools.is_empty())?;
    Some(bytes_div4(serde_json::to_string(tools).ok()?.len()))
}

/// The `bytes_div4_v1` figure of one skill of a `skills/list` reply: a
/// quarter of the bytes of its name and description.
pub fn codex_skill_tokens(name: &str, description: &str) -> u64 {
    bytes_div4(name.len() + description.len())
}

fn reply_figure(entry: &Value, source: &str, at: Option<&str>) -> Option<ChatToolTokens> {
    Some(ChatToolTokens { value: entry.get("tokens")?.as_u64()?, source: source.to_owned(), at: at.map(str::to_owned) })
}

fn unit(kind: ChatToolKind, id: &str, label: &str, group: &str) -> ChatToolItem {
    ChatToolItem {
        kind,
        id: id.to_owned(),
        label: label.to_owned(),
        group: group.to_owned(),
        state: ChatToolState::On,
        fixed_reason: None,
        seen: true,
        tokens: None,
    }
}

fn fixed(item: ChatToolItem, reason: &str) -> ChatToolItem {
    ChatToolItem { state: ChatToolState::Fixed, fixed_reason: Some(reason.to_owned()), ..item }
}

fn switch(item: ChatToolItem, on: bool) -> ChatToolItem {
    ChatToolItem { state: if on { ChatToolState::On } else { ChatToolState::Off }, ..item }
}

fn unseen(item: ChatToolItem) -> ChatToolItem {
    ChatToolItem { seen: false, ..item }
}

/// A server on an off list: the Bridge server is fixed, and a server that
/// the list does not name is on.
fn server_state(item: ChatToolItem, off: &BTreeSet<&str>) -> ChatToolItem {
    if is_bridge(&item.id) {
        return fixed(item, BRIDGE_REASON);
    }
    let on = !off.contains(item.id.as_str());
    switch(item, on)
}

/// Saved off names that the source does not list stay visible.
fn saved_servers(off: &BTreeSet<&str>, listed: &BTreeSet<&str>, group: fn(&str) -> &str) -> Vec<ChatToolItem> {
    off.iter()
        .filter(|name| !listed.contains(*name) && !is_bridge(name))
        .map(|name| unseen(switch(unit(ChatToolKind::Server, name, name, group(name)), false)))
        .collect()
}

fn off_set(row: Option<&ToolSelectionRow>, pick: fn(&ToolSelectionRow) -> &Vec<String>) -> BTreeSet<&str> {
    row.map(|row| pick(row).iter().map(String::as_str).collect()).unwrap_or_default()
}

/// The names of the last Claude init of a chat, the connector catalog and
/// the figures of the latest context reading.
#[derive(Debug, Clone, Default)]
pub struct ClaudeSource {
    pub servers: Vec<String>,
    pub skills: Vec<String>,
    pub plugins: Vec<String>,
    pub catalog: Vec<ConnectorDescriptor>,
    pub figures: Option<TokenFigures>,
}

impl ClaudeSource {
    /// The server, skill and plugin names of the data of an init message.
    pub fn from_init(init: &Value) -> Self {
        let names = |key: &str| -> Vec<String> {
            init.get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("name").and_then(Value::as_str).or_else(|| item.as_str()))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            servers: names("mcp_servers"),
            skills: names("skills"),
            plugins: names("plugins"),
            ..Self::default()
        }
    }
}

fn claude_server_group(name: &str) -> &str {
    if is_bridge(name) {
        "Bridge"
    } else if name.starts_with("claude.ai ") {
        "claude.ai"
    } else if let Some(plugin) = name.strip_prefix("plugin:").and_then(|rest| rest.split_once(':')) {
        plugin.0
    } else {
        "MCP servers"
    }
}

fn claude_skill_group(name: &str) -> &str {
    name.split_once(':').map_or("Skills", |(plugin, _)| plugin)
}

/// The listed skill that also keeps `name`: the SDK keeps a skill whose name
/// is a kept name or ends with `:` and a kept name. The shortest one wins.
fn claude_skill_leader<'a>(name: &str, skills: &'a [String]) -> Option<&'a str> {
    skills
        .iter()
        .map(String::as_str)
        .filter(|lead| claude_sdk_accepts(lead) && name.strip_suffix(lead).is_some_and(|rest| rest.ends_with(':')))
        .min_by_key(|lead| lead.len())
}

/// Claude units: each MCP server and each skill. Servers use the off list,
/// and skills use the allowlist, so a skill that appears later is left out.
/// A skill that a shorter listed name also keeps follows that name.
pub fn claude_units(source: &ClaudeSource, row: Option<&ToolSelectionRow>) -> Vec<ChatToolItem> {
    let row = row.filter(|row| row.harness == "claude");
    let off = off_set(row, |row| &row.servers_off);
    let labels: BTreeMap<&str, &str> = source
        .catalog
        .iter()
        .filter(|connector| connector.harness.as_deref() == Some("claude"))
        .filter_map(|connector| Some((connector.server.as_deref()?, connector.display_name.as_str())))
        .collect();
    let mut items = Vec::new();
    let mut listed = BTreeSet::new();
    for name in source.servers.iter().map(String::as_str).chain(labels.keys().copied()) {
        if !listed.insert(name) {
            continue;
        }
        let label = labels.get(name).copied().unwrap_or(name);
        let item = unit(ChatToolKind::Server, name, label, claude_server_group(name));
        let item = ChatToolItem { tokens: figure(source.figures.as_ref(), false, name), ..item };
        items.push(server_state(item, &off));
    }
    items.extend(saved_servers(&off, &listed, claude_server_group));
    let only: Option<BTreeSet<&str>> = row
        .and_then(|row| row.claude_skills.as_ref())
        .map(|skills| skills.only.iter().map(String::as_str).collect());
    let mut skills = BTreeSet::new();
    for name in &source.skills {
        if !skills.insert(name.as_str()) {
            continue;
        }
        let item = unit(ChatToolKind::Skill, name, name, claude_skill_group(name));
        let item = ChatToolItem { tokens: figure(source.figures.as_ref(), true, name), ..item };
        items.push(if !claude_sdk_accepts(name) {
            fixed(item, CLAUDE_SKILL_NAME_REASON)
        } else if let Some(lead) = claude_skill_leader(name, &source.skills) {
            fixed(item, &format!("This skill follows the skill {lead}. The Claude SDK keeps it when the chat keeps {lead}."))
        } else {
            switch(item, only.as_ref().is_none_or(|kept| kept.contains(name.as_str())))
        });
    }
    for name in only.iter().flatten().filter(|name| !skills.contains(*name)) {
        items.push(unseen(unit(ChatToolKind::Skill, name, name, claude_skill_group(name))));
    }
    let mut plugins = BTreeSet::new();
    for name in &source.plugins {
        if plugins.insert(name.as_str()) {
            items.push(fixed(unit(ChatToolKind::Plugin, name, name, "Plugins"), CLAUDE_PLUGIN_REASON));
        }
    }
    items
}

/// The replies of the app-control path of a Codex chat, as
/// `apps::codex::translate` gives them. `None` is a read that did not answer.
#[derive(Debug, Clone, Default)]
pub struct CodexSource {
    /// The `servers` reply: `{servers:[{name, status, connectors, tokens}]}`.
    pub status: Option<Value>,
    /// The `skills` reply: `{skills:[{name, enabled, tokens}]}`.
    pub skills: Option<Value>,
    /// The `config` reply: `{userSkillsKey, plugins, files}`.
    pub config: Option<Value>,
    /// Bridge cannot confirm the `config` fact for the next launch.
    pub unconfirmed: bool,
    /// When the runtime answered.
    pub at: Option<String>,
}

fn reply_list<'a>(reply: Option<&'a Value>, key: &str) -> &'a [Value] {
    reply.and_then(|reply| reply.get(key)).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The apps inside `codex_apps`: the connectors of its reply entry.
fn codex_apps_inside(server: Option<&Value>) -> Vec<String> {
    let names: BTreeSet<&str> = reply_list(server, "connectors").iter().filter_map(Value::as_str).collect();
    names.into_iter().map(str::to_owned).collect()
}

/// `None` when the skills are units. Otherwise the reason that they are
/// fixed: a `skills` key of the user, a read that Bridge cannot confirm for
/// the next launch, or a config that Bridge has not read.
fn codex_skill_reason(source: &CodexSource) -> Option<&'static str> {
    match source.config.as_ref().and_then(|reply| reply.get("userSkillsKey")).and_then(Value::as_bool) {
        Some(true) => Some(CODEX_SKILLS_TABLE_REASON),
        _ if source.unconfirmed => Some(CODEX_SKILLS_UNCONFIRMED_REASON),
        Some(false) => None,
        None => Some(CODEX_SKILLS_UNKNOWN_REASON),
    }
}

/// Codex units: each MCP server of the effective config, `codex_apps` as one
/// switch, and skills only when the config has no `skills` key of the user.
pub fn codex_units(source: &CodexSource, row: Option<&ToolSelectionRow>) -> Vec<ChatToolItem> {
    let row = row.filter(|row| row.harness == "codex");
    let at = source.at.as_deref();
    let off = off_set(row, |row| &row.servers_off);
    let servers = reply_list(source.status.as_ref(), "servers");
    let mut items = Vec::new();
    let mut listed = BTreeSet::new();
    let mut apps = None;
    for server in servers {
        let Some(name) = server.get("name").and_then(Value::as_str) else { continue };
        if name == CODEX_APPS {
            apps = Some(server);
            continue;
        }
        if !listed.insert(name) {
            continue;
        }
        let item = ChatToolItem {
            tokens: reply_figure(server, CODEX_SERVER_SOURCE, at),
            ..unit(ChatToolKind::Server, name, name, "MCP servers")
        };
        items.push(if is_bridge(name) || codex_key_is_safe(name) { server_state(item, &off) } else { fixed(item, CODEX_KEY_REASON) });
    }
    items.extend(saved_servers(&off, &listed, |_| "MCP servers"));
    let apps_item = ChatToolItem {
        tokens: apps.and_then(|server| reply_figure(server, CODEX_SERVER_SOURCE, at)),
        ..unit(ChatToolKind::Apps, CODEX_APPS, "Apps", "Apps")
    };
    items.push(switch(apps_item, !row.is_some_and(|row| row.codex_apps_off)));
    for app in codex_apps_inside(apps) {
        items.push(fixed(unit(ChatToolKind::App, &app, &app, "Apps"), CODEX_APP_REASON));
    }
    let reason = codex_skill_reason(source);
    let skills_off = off_set(row, |row| &row.codex_skills_off);
    let mut skills = BTreeSet::new();
    for skill in reply_list(source.skills.as_ref(), "skills") {
        let Some(name) = skill.get("name").and_then(Value::as_str) else { continue };
        if !skills.insert(name) {
            continue;
        }
        let item = ChatToolItem {
            tokens: reply_figure(skill, CODEX_SKILL_SOURCE, at),
            ..unit(ChatToolKind::Skill, name, name, "Skills")
        };
        items.push(match reason {
            Some(reason) => fixed(item, reason),
            None => switch(item, !skills_off.contains(name)),
        });
    }
    for name in skills_off.iter().filter(|name| !skills.contains(*name)) {
        let item = unseen(switch(unit(ChatToolKind::Skill, name, name, "Skills"), false));
        items.push(match reason {
            Some(reason) => fixed(item, reason),
            None => item,
        });
    }
    for name in reply_list(source.config.as_ref(), "plugins").iter().filter_map(Value::as_str) {
        items.push(fixed(unit(ChatToolKind::Plugin, name, name, "Plugins"), CODEX_PLUGIN_REASON));
    }
    items
}

/// The reply of the warm OpenCode server of a chat: `GET /mcp` (V1, a map by
/// key) or `/api/mcp` (V2, `{data:[{name}]}`).
#[derive(Debug, Clone, Default)]
pub struct OpenCodeSource {
    pub mcp: Option<Value>,
}

fn opencode_server_names(mcp: Option<&Value>) -> Vec<&str> {
    let Some(mcp) = mcp else { return Vec::new() };
    if let Some(data) = mcp.get("data").and_then(Value::as_array) {
        return data.iter().filter_map(|entry| entry.get("name").and_then(Value::as_str)).collect();
    }
    mcp.as_object().map(|servers| servers.keys().map(String::as_str).collect()).unwrap_or_default()
}

/// OpenCode units: each MCP server. Skills show as one fixed item, and no
/// item has a token figure, because OpenCode reports none.
pub fn opencode_units(source: &OpenCodeSource, row: Option<&ToolSelectionRow>) -> Vec<ChatToolItem> {
    let row = row.filter(|row| row.harness == "opencode");
    let off = off_set(row, |row| &row.servers_off);
    let mut items = Vec::new();
    let mut listed = BTreeSet::new();
    for name in opencode_server_names(source.mcp.as_ref()) {
        if listed.insert(name) {
            let server = unit(ChatToolKind::Server, name, name, "MCP servers");
            items.push(if has_substitution_marker(name) {
                fixed(server, OPENCODE_MARKER_REASON)
            } else {
                server_state(server, &off)
            });
        }
    }
    items.extend(saved_servers(&off, &listed, |_| "MCP servers"));
    items.push(fixed(unit(ChatToolKind::AllSkills, "skills", "Skills", "Skills"), OPENCODE_SKILLS_REASON));
    items
}

/// One Codex value in one of the three forms that [`codex_overrides`] can
/// emit. Tests read each value back through [`parse_codex_off`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexOff {
    Server(String),
    Apps,
    Skills(Vec<String>),
}

/// The form of one Codex value, or `None` for any other value. The parse is
/// strict: each skill entry is `{name="<TOML basic string>",enabled=false}`.
#[cfg(test)]
pub(crate) fn parse_codex_off(value: &str) -> Option<CodexOff> {
    if value == CODEX_APPS_OFF {
        return Some(CodexOff::Apps);
    }
    if let Some(key) = value.strip_prefix("mcp_servers.").and_then(|rest| rest.strip_suffix(".enabled=false")) {
        return codex_key_is_safe(key).then(|| CodexOff::Server(key.to_owned()));
    }
    let mut rest = value.strip_prefix("skills.config=[")?.strip_suffix(']')?;
    let mut names = Vec::new();
    loop {
        rest = rest.strip_prefix("{name=\"")?;
        let mut name = String::new();
        let mut chars = rest.char_indices();
        let end = loop {
            let (at, ch) = chars.next()?;
            match ch {
                '"' => break at,
                '\\' => match chars.next()?.1 {
                    '"' => name.push('"'),
                    '\\' => name.push('\\'),
                    'u' => {
                        let hex: String = (0..4).map(|_| chars.next().map(|(_, digit)| digit)).collect::<Option<_>>()?;
                        name.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    _ => return None,
                },
                ch if ch.is_control() => return None,
                ch => name.push(ch),
            }
        };
        rest = rest.get(end + 1..)?.strip_prefix(",enabled=false}")?;
        names.push(name);
        if rest.is_empty() {
            return Some(CodexOff::Skills(names));
        }
        rest = rest.strip_prefix(',')?;
    }
}

pub mod picker;

#[cfg(test)]
mod tests;
