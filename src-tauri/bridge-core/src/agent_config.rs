use crate::{delegation::Effort, opencode_adapter::OpenCodeSettings, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_PROMPT_BYTES: usize = 64 * 1024;
const VALID_HARNESSES: [&str; 4] = ["bridge", "codex", "claude", "opencode"];
const VALID_ROLES: [&str; 6] = [
    "orchestrator",
    "research",
    "implementation",
    "verification",
    "planning",
    "documentation",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessConfig {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default = "empty_object")]
    pub advanced: Value,
    #[serde(default)]
    pub is_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDefinition {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub role: String,
    pub harness: String,
    #[serde(default)]
    pub model: Option<String>,
    pub effort: Effort,
    #[serde(default)]
    pub system_prompt: String,
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub is_built_in: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// How much Bridge asks before an agent acts.
///
/// One switch in this slice. Fields added later (auto-allow workers, inherited
/// grants, command allowlists) must be `#[serde(default)]` so a policy written by
/// an older build still reads — the stored payload is durable and outlives the
/// binary that wrote it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionPolicy {
    /// Auto-accept every provider approval, for every agent. The two structural
    /// gates — worker write scope and browser outward effects — are unaffected;
    /// they are authorization, not convenience.
    pub bypass_all: bool,
    pub updated_at: String,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        // A fresh install asks. Nothing is granted until someone opts in.
        Self {
            bypass_all: false,
            updated_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigState {
    pub harnesses: Vec<HarnessConfig>,
    pub agents: Vec<AgentDefinition>,
    pub default_agent_id: String,
    pub permission_policy: PermissionPolicy,
}

fn empty_object() -> Value {
    json!({})
}

fn default_harnesses() -> Vec<HarnessConfig> {
    [
        ("bridge", "Bridge"),
        ("codex", "Codex"),
        ("claude", "Claude Code"),
        ("opencode", "OpenCode"),
    ]
    .into_iter()
    .map(|(id, label)| HarnessConfig {
        id: id.into(),
        label: label.into(),
        enabled: true,
        default_model: None,
        effort: None,
        system_prompt: String::new(),
        advanced: empty_object(),
        is_override: false,
    })
    .collect()
}

fn built_in_agents() -> Vec<AgentDefinition> {
    [
        (
            "bridge-orchestrator",
            "Bridge orchestrator",
            "Plans, routes, and owns the final answer.",
            "orchestrator",
            Effort::Medium,
        ),
        (
            "bridge-research",
            "Research agent",
            "Collects scoped evidence and findings.",
            "research",
            Effort::Medium,
        ),
        (
            "bridge-implementation",
            "Implementation agent",
            "Makes focused code changes.",
            "implementation",
            Effort::Medium,
        ),
        (
            "bridge-verification",
            "Verification agent",
            "Tests outcomes independently.",
            "verification",
            Effort::High,
        ),
        (
            "bridge-planning",
            "Planning agent",
            "Turns ambiguous work into an executable plan.",
            "planning",
            Effort::High,
        ),
        (
            "bridge-documentation",
            "Documentation agent",
            "Produces concise project documentation.",
            "documentation",
            Effort::Low,
        ),
    ]
    .into_iter()
    .map(|(id, name, description, role, effort)| AgentDefinition {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        role: role.into(),
        harness: "bridge".into(),
        model: None,
        effort,
        system_prompt: String::new(),
        enabled: true,
        is_default: id == "bridge-orchestrator",
        is_built_in: true,
        created_at: String::new(),
        updated_at: String::new(),
    })
    .collect()
}

fn validate_prompt(value: &str) -> Result<(), BridgeError> {
    if value.len() > MAX_PROMPT_BYTES {
        return Err(BridgeError::Invalid(
            "system prompt exceeds the 64 KiB limit".into(),
        ));
    }
    Ok(())
}

fn validate_harness(config: &HarnessConfig) -> Result<(), BridgeError> {
    if !VALID_HARNESSES.contains(&config.id.as_str()) {
        return Err(BridgeError::Invalid(format!(
            "unknown harness {}",
            config.id
        )));
    }
    if !config.advanced.is_object() {
        return Err(BridgeError::Invalid(
            "advanced harness configuration must be a JSON object".into(),
        ));
    }
    if config.id == "opencode" {
        opencode_settings(Some(config))?;
    }
    validate_prompt(&config.system_prompt)
}

pub fn opencode_settings(config: Option<&HarnessConfig>) -> Result<OpenCodeSettings, BridgeError> {
    let Some(config) = config else {
        return Ok(OpenCodeSettings::default());
    };
    let mut settings: OpenCodeSettings =
        serde_json::from_value(config.advanced.clone()).map_err(|error| {
            BridgeError::Invalid(format!("invalid OpenCode advanced configuration: {error}"))
        })?;
    settings.executable_path = settings
        .executable_path
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let mut seen = std::collections::HashSet::new();
    for model in &mut settings.visible_models {
        *model = model.trim().to_owned();
        let qualified = model
            .split_once('/')
            .is_some_and(|(provider, name)| !provider.is_empty() && !name.is_empty());
        if !qualified {
            return Err(BridgeError::Invalid(format!(
                "OpenCode visible model must be provider-qualified: {model:?}"
            )));
        }
        if !seen.insert(model.clone()) {
            return Err(BridgeError::Invalid(format!(
                "OpenCode visible model is duplicated: {model}"
            )));
        }
    }
    Ok(settings)
}

fn validate_agent(agent: &AgentDefinition) -> Result<(), BridgeError> {
    if agent.name.trim().is_empty() {
        return Err(BridgeError::Invalid("agent name cannot be empty".into()));
    }
    if !VALID_ROLES.contains(&agent.role.as_str()) {
        return Err(BridgeError::Invalid(format!(
            "unknown agent role {}",
            agent.role
        )));
    }
    if !VALID_HARNESSES.contains(&agent.harness.as_str()) {
        return Err(BridgeError::Invalid(format!(
            "unknown agent harness {}",
            agent.harness
        )));
    }
    validate_prompt(&agent.system_prompt)
}

fn stored<T: for<'de> Deserialize<'de>>(
    db: &Connection,
    kind: &str,
    id: &str,
) -> Result<Option<T>, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![kind, id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                BridgeError::Invalid(format!("invalid stored configuration: {error}"))
            })
        })
        .transpose()
}

fn upsert<T: Serialize>(
    db: &Connection,
    kind: &str,
    id: &str,
    value: &T,
) -> Result<(), BridgeError> {
    let now = Utc::now().to_rfc3339();
    let payload =
        serde_json::to_string(value).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![kind, id, payload, now],
    )?;
    Ok(())
}

fn default_agent_id(db: &Connection) -> Result<String, BridgeError> {
    Ok(stored::<Value>(db, "meta", "defaults")?
        .and_then(|value| {
            value
                .get("defaultAgentId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "bridge-orchestrator".into()))
}

pub fn state(db: &Connection) -> Result<ConfigState, BridgeError> {
    let mut harnesses = default_harnesses();
    for config in &mut harnesses {
        if let Some(mut value) = stored::<HarnessConfig>(db, "harness", &config.id)? {
            value.is_override = true;
            *config = value;
        }
    }
    let mut agents = built_in_agents();
    for agent in &mut agents {
        if let Some(mut value) = stored::<AgentDefinition>(db, "agent", &agent.id)? {
            value.is_built_in = true;
            *agent = value;
        }
    }
    let mut statement = db.prepare(
        "SELECT payload FROM configuration_entries WHERE kind='agent' ORDER BY created_at,id",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        let value: AgentDefinition = serde_json::from_str(&row?)
            .map_err(|error| BridgeError::Invalid(format!("invalid stored agent: {error}")))?;
        if !agents.iter().any(|agent| agent.id == value.id) {
            agents.push(value);
        }
    }
    let mut selected = default_agent_id(db)?;
    if !agents
        .iter()
        .any(|agent| agent.id == selected && agent.enabled && agent.role == "orchestrator")
    {
        selected = "bridge-orchestrator".into();
    }
    for agent in &mut agents {
        agent.is_default = agent.id == selected;
    }
    Ok(ConfigState {
        harnesses,
        agents,
        default_agent_id: selected,
        permission_policy: permission_policy(db)?,
    })
}

/// The stored policy, or the asking default.
///
/// Read on every provider approval, so it is a single primary-key lookup on
/// `configuration_entries` and nothing more.
pub fn permission_policy(db: &Connection) -> Result<PermissionPolicy, BridgeError> {
    Ok(stored::<PermissionPolicy>(db, "permission_policy", "global")?.unwrap_or_default())
}

pub fn save_permission_policy(
    db: &Connection,
    mut policy: PermissionPolicy,
) -> Result<ConfigState, BridgeError> {
    // Stamped host-side: the client does not get to claim when a policy changed,
    // and the settings page renders what was actually stored.
    policy.updated_at = Utc::now().to_rfc3339();
    upsert(db, "permission_policy", "global", &policy)?;
    state(db)
}

pub fn save_harness(
    db: &Connection,
    mut config: HarnessConfig,
) -> Result<ConfigState, BridgeError> {
    validate_harness(&config)?;
    config.is_override = true;
    let id = config.id.clone();
    upsert(db, "harness", &id, &config)?;
    state(db)
}

pub fn reset_harness(db: &Connection, id: &str) -> Result<ConfigState, BridgeError> {
    if !VALID_HARNESSES.contains(&id) {
        return Err(BridgeError::Invalid(format!("unknown harness {id}")));
    }
    db.execute(
        "DELETE FROM configuration_entries WHERE kind='harness' AND id=?1",
        params![id],
    )?;
    state(db)
}

pub fn save_agent(db: &Connection, mut agent: AgentDefinition) -> Result<ConfigState, BridgeError> {
    if agent.id.trim().is_empty() {
        agent.id = format!("custom-{}", Uuid::new_v4().simple());
    }
    validate_agent(&agent)?;
    let built_in = built_in_agents().iter().any(|item| item.id == agent.id);
    let existing: Option<AgentDefinition> = stored(db, "agent", &agent.id)?;
    let now = Utc::now().to_rfc3339();
    agent.is_built_in = built_in;
    agent.created_at = existing
        .as_ref()
        .map(|value| value.created_at.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| now.clone());
    agent.updated_at = now;
    agent.is_default = false;
    let id = agent.id.clone();
    upsert(db, "agent", &id, &agent)?;
    state(db)
}

pub fn delete_agent(db: &Connection, id: &str) -> Result<ConfigState, BridgeError> {
    db.execute(
        "DELETE FROM configuration_entries WHERE kind='agent' AND id=?1",
        params![id],
    )?;
    if default_agent_id(db)? == id {
        set_default(db, "bridge-orchestrator")?;
    }
    state(db)
}

pub fn set_default(db: &Connection, id: &str) -> Result<ConfigState, BridgeError> {
    let current = state(db)?;
    if !current
        .agents
        .iter()
        .any(|agent| agent.id == id && agent.enabled && agent.role == "orchestrator")
    {
        return Err(BridgeError::Invalid(
            "default agent must be an enabled orchestrator".into(),
        ));
    }
    upsert(db, "meta", "defaults", &json!({"defaultAgentId": id}))?;
    state(db)
}

pub fn reset_all(db: &Connection) -> Result<ConfigState, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    crate::prompt_sections::append_reset_all_revisions(&transaction)?;
    transaction.execute("DELETE FROM configuration_entries", [])?;
    transaction.commit()?;
    state(db)
}

pub fn harness_prompt(db: &Connection, harness: &str) -> String {
    state(db)
        .ok()
        .and_then(|state| state.harnesses.into_iter().find(|item| item.id == harness))
        .filter(|item| item.enabled)
        .map(|item| item.system_prompt)
        .unwrap_or_default()
}

pub fn harness_config(db: &Connection, harness: &str) -> Option<HarnessConfig> {
    state(db)
        .ok()
        .and_then(|state| state.harnesses.into_iter().find(|item| item.id == harness))
        .filter(|item| item.enabled)
}

pub fn is_harness_enabled(db: &Connection, harness: &str) -> bool {
    state(db)
        .ok()
        .and_then(|state| state.harnesses.into_iter().find(|item| item.id == harness))
        .map(|item| item.enabled)
        .unwrap_or(true)
}

pub fn role_prompt(db: &Connection, role: &str) -> String {
    state(db)
        .ok()
        .and_then(|state| {
            state
                .agents
                .into_iter()
                .find(|item| item.enabled && item.role == role)
        })
        .map(|item| item.system_prompt)
        .unwrap_or_default()
}

pub fn default_orchestrator(db: &Connection) -> Option<AgentDefinition> {
    state(db).ok().and_then(|state| {
        state
            .agents
            .into_iter()
            .find(|item| item.id == state.default_agent_id)
    })
}

pub fn prompt_suffix(db: &Connection, harness: &str, role: &str) -> String {
    [
        harness_prompt(db, "bridge"),
        harness_prompt(db, harness),
        role_prompt(db, role),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n\n")
}

pub fn orchestrator_prompt(db: &Connection, harness: &str) -> String {
    let agent_prompt = default_orchestrator(db)
        .map(|agent| agent.system_prompt)
        .unwrap_or_default();
    [
        harness_prompt(db, "bridge"),
        harness_prompt(db, harness),
        agent_prompt,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n\n")
}

pub fn session_prompt(db: &Connection, harness: &str) -> String {
    [harness_prompt(db, "bridge"), harness_prompt(db, harness)]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    /// A fresh install asks. This is the assertion that keeps a shipping default
    /// from silently granting every agent everything.
    #[test]
    fn permission_policy_defaults_to_asking() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(permission_policy(&db).unwrap(), PermissionPolicy::default());
        assert!(!permission_policy(&db).unwrap().bypass_all);
        assert!(!state(&db).unwrap().permission_policy.bypass_all);
    }

    #[test]
    fn permission_policy_round_trips_through_the_config_store() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let saved = save_permission_policy(
            &db,
            PermissionPolicy {
                bypass_all: true,
                // Claimed by the client and ignored: the host stamps it.
                updated_at: "whenever-i-say".into(),
            },
        )
        .unwrap();

        assert!(saved.permission_policy.bypass_all);
        assert_ne!(saved.permission_policy.updated_at, "whenever-i-say");
        assert!(!saved.permission_policy.updated_at.is_empty());
        // Re-read from durable state, not from the returned value.
        assert!(permission_policy(&db).unwrap().bypass_all);
        assert!(state(&db).unwrap().permission_policy.bypass_all);

        save_permission_policy(&db, PermissionPolicy::default()).unwrap();
        assert!(!permission_policy(&db).unwrap().bypass_all);
    }

    #[test]
    fn saving_a_policy_leaves_the_rest_of_config_state_alone() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let before = state(&db).unwrap();

        let after = save_permission_policy(
            &db,
            PermissionPolicy {
                bypass_all: true,
                updated_at: String::new(),
            },
        )
        .unwrap();

        assert_eq!(after.harnesses, before.harnesses);
        assert_eq!(after.agents, before.agents);
        assert_eq!(after.default_agent_id, before.default_agent_id);
    }

    /// Slice 3 adds fields to this policy. A payload written by today's build has
    /// to keep reading then, and one written by a newer build has to keep reading
    /// on an older one — hence `default` on the container, not just the fields.
    #[test]
    fn a_policy_payload_survives_fields_it_does_not_know() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        upsert(
            &db,
            "permission_policy",
            "global",
            &json!({"bypassAll": true, "autoAllowWorkers": true, "updatedAt": "now"}),
        )
        .unwrap();

        let policy = permission_policy(&db).unwrap();
        assert!(
            policy.bypass_all,
            "an unknown future field must not discard the whole policy"
        );
    }

    #[test]
    fn custom_agents_round_trip_and_can_be_deleted() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let agent = AgentDefinition {
            id: String::new(),
            name: "Release captain".into(),
            description: String::new(),
            role: "orchestrator".into(),
            harness: "claude".into(),
            model: Some("model-x".into()),
            effort: Effort::High,
            system_prompt: "Own release quality.".into(),
            enabled: true,
            is_default: false,
            is_built_in: false,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let saved = save_agent(&db, agent).unwrap();
        let custom = saved
            .agents
            .iter()
            .find(|item| item.name == "Release captain")
            .unwrap();
        let selected = set_default(&db, &custom.id).unwrap();
        assert_eq!(selected.default_agent_id, custom.id);
        let reset = delete_agent(&db, &custom.id).unwrap();
        assert_eq!(reset.default_agent_id, "bridge-orchestrator");
        assert!(!reset
            .agents
            .iter()
            .any(|item| item.name == "Release captain"));
    }

    #[test]
    fn built_in_delete_restores_defaults_and_prompts_compose() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let mut bridge = state(&db)
            .unwrap()
            .harnesses
            .into_iter()
            .find(|item| item.id == "bridge")
            .unwrap();
        bridge.system_prompt = "Bridge-wide rule".into();
        save_harness(&db, bridge).unwrap();
        let mut worker = state(&db)
            .unwrap()
            .agents
            .into_iter()
            .find(|item| item.id == "bridge-research")
            .unwrap();
        worker.system_prompt = "Research rule".into();
        save_agent(&db, worker).unwrap();
        assert_eq!(
            prompt_suffix(&db, "codex", "research"),
            "Bridge-wide rule\n\nResearch rule"
        );
        let reset = delete_agent(&db, "bridge-research").unwrap();
        assert!(reset
            .agents
            .iter()
            .find(|item| item.id == "bridge-research")
            .unwrap()
            .system_prompt
            .is_empty());
    }

    #[test]
    fn disabled_harness_is_not_available_for_new_work() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let mut codex = state(&db)
            .unwrap()
            .harnesses
            .into_iter()
            .find(|item| item.id == "codex")
            .unwrap();
        codex.enabled = false;
        save_harness(&db, codex).unwrap();
        assert!(!is_harness_enabled(&db, "codex"));
        assert!(harness_config(&db, "codex").is_none());
    }

    #[test]
    fn opencode_advanced_config_rejects_invalid_shapes_and_secret_fields() {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        let mut opencode = state(&db)
            .unwrap()
            .harnesses
            .into_iter()
            .find(|item| item.id == "opencode")
            .unwrap();

        opencode.advanced = json!({
            "executablePath": " /managed/opencode ",
            "visibleModels": ["opencode-go/kimi-k2.5"]
        });
        let settings = opencode_settings(Some(&opencode)).unwrap();
        assert_eq!(
            settings.executable_path.as_deref(),
            Some("/managed/opencode")
        );
        assert_eq!(settings.visible_models, ["opencode-go/kimi-k2.5"]);

        for invalid in [
            json!({"executablePath": 42}),
            json!({"visibleModels": ["unqualified"]}),
            json!({"apiKey": "must-never-be-persisted"}),
            json!({"token": "must-never-be-persisted"}),
        ] {
            opencode.advanced = invalid;
            assert!(save_harness(&db, opencode.clone()).is_err());
        }
    }
}
