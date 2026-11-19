//! The tool choice of a chat: the connectors and skills that its launches
//! leave out. Bridge stores only the choice (kind `chat_tool_selection_v1`),
//! never an inventory. A choice only removes items, so nothing here can add
//! an allow rule, remove a deny or change a permission.
use crate::{chat_defaults, provider_accounts, BridgeError};
use bridge_protocol::messages::{
    ChatModelChoice, ChatToolItem, ChatToolKind, ChatToolSelection, ChatToolSelectionStatus,
    ChatToolState, ChatToolTokens, ClaudeSkillChoice, ConnectorDescriptor,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const KIND: &str = "chat_tool_selection_v1";
/// Half of the 128 KiB cap of `chat_defaults::read`, so a stored row always
/// reads back.
pub const MAX_ROW_BYTES: usize = 64 * 1024;
pub const CODEX_APPS: &str = "codex_apps";
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
const OPENCODE_SKILLS_REASON: &str = "OpenCode has no verified switch for one skill.";

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
    !(name.trim().is_empty()
        || name.trim() != name
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
        }
    }
    Ok(ChatToolSelection { account, ..choice.clone() })
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
            effective_hash,
        }
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
}

/// The tool launch of a chat: `None` for a chat with no stored choice, and
/// `None` unless the session is an interactive top-level chat outside a
/// workflow and its stored choice applies. Workers, briefings and workflows
/// never read the row.
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
    Ok(status(&row, target.harness, account.as_deref())
        .applies
        .then(|| ToolLaunch::from_row(&row)))
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

fn estimate(bytes: usize, source: &str, at: Option<&str>) -> ChatToolTokens {
    ChatToolTokens {
        value: u64::try_from(bytes / 4).unwrap_or(u64::MAX),
        source: source.to_owned(),
        at: at.map(str::to_owned),
    }
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

/// Claude units: each MCP server and each skill. Servers use the off list,
/// and skills use the allowlist, so a skill that appears later is left out.
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
        items.push(if claude_sdk_accepts(name) {
            switch(item, only.as_ref().is_none_or(|kept| kept.contains(name.as_str())))
        } else {
            fixed(item, CLAUDE_SKILL_NAME_REASON)
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

/// The replies of the warm Codex runtime of a chat. `None` is a read that
/// did not answer.
#[derive(Debug, Clone, Default)]
pub struct CodexSource {
    /// The `mcpServerStatus/list` result for the chat thread.
    pub status: Option<Value>,
    /// The `skills/list` result.
    pub skills: Option<Value>,
    /// The `config/read` result.
    pub config: Option<Value>,
    /// When the runtime answered.
    pub at: Option<String>,
}

fn tools_estimate(server: &Value, at: Option<&str>) -> Option<ChatToolTokens> {
    let tools = server.get("tools")?.as_object().filter(|tools| !tools.is_empty())?;
    let bytes = serde_json::to_string(tools).ok()?.len();
    Some(estimate(bytes, CODEX_SERVER_SOURCE, at))
}

/// The apps inside `codex_apps`: it names its tools `<app>.<tool>`.
fn codex_apps_inside(server: Option<&Value>) -> Vec<String> {
    let names: BTreeSet<&str> = server
        .and_then(|server| server.get("tools"))
        .and_then(Value::as_object)
        .map(|tools| tools.keys().filter_map(|key| key.split_once('.').map(|(app, _)| app)).collect())
        .unwrap_or_default();
    names.into_iter().map(str::to_owned).collect()
}

/// `None` when the skills are units. Otherwise the reason that they are
/// fixed: a `skills` key in the config, or a config that Bridge has not read.
fn codex_skill_reason(config: Option<&Value>) -> Option<&'static str> {
    let table = config
        .and_then(|reply| reply.get("config"))
        .and_then(Value::as_object)
        .map(|config| config.contains_key("skills"));
    match table {
        Some(false) => None,
        Some(true) => Some(CODEX_SKILLS_TABLE_REASON),
        None => Some(CODEX_SKILLS_UNKNOWN_REASON),
    }
}

/// Codex units: each MCP server of the effective config, `codex_apps` as one
/// switch, and skills only when the config has no `skills` key.
pub fn codex_units(source: &CodexSource, row: Option<&ToolSelectionRow>) -> Vec<ChatToolItem> {
    let row = row.filter(|row| row.harness == "codex");
    let at = source.at.as_deref();
    let off = off_set(row, |row| &row.servers_off);
    let servers = source
        .status
        .as_ref()
        .and_then(|status| status.get("data"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
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
        let item = ChatToolItem { tokens: tools_estimate(server, at), ..unit(ChatToolKind::Server, name, name, "MCP servers") };
        items.push(if is_bridge(name) || codex_key_is_safe(name) { server_state(item, &off) } else { fixed(item, CODEX_KEY_REASON) });
    }
    items.extend(saved_servers(&off, &listed, |_| "MCP servers"));
    let apps_item = ChatToolItem {
        tokens: apps.and_then(|server| tools_estimate(server, at)),
        ..unit(ChatToolKind::Apps, CODEX_APPS, "Apps", "Apps")
    };
    items.push(switch(apps_item, !row.is_some_and(|row| row.codex_apps_off)));
    for app in codex_apps_inside(apps) {
        items.push(fixed(unit(ChatToolKind::App, &app, &app, "Apps"), CODEX_APP_REASON));
    }
    let reason = codex_skill_reason(source.config.as_ref());
    let skills_off = off_set(row, |row| &row.codex_skills_off);
    let mut skills = BTreeSet::new();
    let listed_skills = source
        .skills
        .as_ref()
        .and_then(|reply| reply.get("data"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("skills").and_then(Value::as_array))
        .flatten();
    for skill in listed_skills {
        let Some(name) = skill.get("name").and_then(Value::as_str) else { continue };
        if !skills.insert(name) {
            continue;
        }
        let description = skill.get("description").and_then(Value::as_str).unwrap_or_default();
        let item = ChatToolItem {
            tokens: Some(estimate(name.len() + description.len(), CODEX_SKILL_SOURCE, at)),
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
    let plugins = source
        .config
        .as_ref()
        .and_then(|reply| reply.get("config"))
        .and_then(|config| config.get("plugins"))
        .and_then(Value::as_object);
    for name in plugins.into_iter().flat_map(|plugins| plugins.keys()) {
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
            items.push(server_state(unit(ChatToolKind::Server, name, name, "MCP servers"), &off));
        }
    }
    items.extend(saved_servers(&off, &listed, |_| "MCP servers"));
    items.push(fixed(unit(ChatToolKind::AllSkills, "skills", "Skills", "Skills"), OPENCODE_SKILLS_REASON));
    items
}

#[cfg(test)]
mod tests;
