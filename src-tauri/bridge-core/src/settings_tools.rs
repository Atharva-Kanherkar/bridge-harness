//! The settings tools, core side: a chat reads and changes Bridge's own
//! settings ("make the theme light", "make Opus the default model").
//!
//! Calls arrive on the chat-tools route with the chat's token (see
//! `notes::tool`), and only a top-level chat may use them: a worker reports to
//! its orchestrator and has no business restyling the person's app.
//!
//! The catalog is deliberately closed. Every setting here changes how Bridge
//! looks or what a new chat starts on. Nothing here can widen what an agent
//! may do: default access, permissions, API keys, accounts, budgets, worker
//! limits and approvals are not in it, and a request for one is refused with
//! a pointer to Settings. Each write goes through the same validated path the
//! Settings screen uses.

use std::sync::Arc;

use bridge_protocol::messages::{
    ActiveTurnInput, AppPreferenceValues, ChatDefaults, ChatEffortChoice, ChatModelChoice, EffortSelectorStyle,
    ResolveChatSettingsParams, SaveChatDefaultsParams, ThemePreference, ThemeSkin,
};
use serde_json::{json, Value};

use crate::model::AdapterDescriptor;
use crate::notes::tool::{bad, ToolError};
use crate::runtime::BridgeCore;

pub const READ_TOOL: &str = "settings_read";
pub const CHANGE_TOOL: &str = "settings_change";
pub const TOOLS: [&str; 2] = [READ_TOOL, CHANGE_TOOL];

/// What is missing from the catalog, and why. Shown with every listing and
/// with every refusal, so the agent can tell the person where to go.
pub const NOT_HERE: &str = "Default access (Ask or Full access), permissions, API keys, accounts, budgets, worker limits and approvals are not settings you can change: an agent must never widen its own authority. Tell the person to change those in Bridge Settings.";

/// Most models a harness lists before `search` is needed to see the rest.
const MODEL_LIST_CAP: usize = 40;

#[derive(Clone, Copy)]
enum Kind {
    Choice(&'static [&'static str]),
    Switch,
    Model,
    Effort,
}

struct Setting {
    key: &'static str,
    kind: Kind,
    default: Value,
    description: &'static str,
    applies: &'static str,
}

const NOW: &str = "every open Bridge window, at once";
const NEW_CHATS: &str = "new chats from now on; chats already open keep theirs";

fn catalog() -> Vec<Setting> {
    vec![
        Setting { key: "appearance.theme", kind: Kind::Choice(&["system", "light", "dark"]), default: json!("system"), description: "Light or dark mode. system follows macOS appearance.", applies: NOW },
        Setting { key: "appearance.skin", kind: Kind::Choice(&["graphite", "vibrancy", "omarchy"]), default: json!("graphite"), description: "The character of the window chrome: graphite is opaque, vibrancy lets the desktop wallpaper tint the sidebar, omarchy follows the Omarchy theme on Linux.", applies: NOW },
        Setting { key: "appearance.effortSelector", kind: Kind::Choice(&["slider", "sentence", "list"]), default: json!("slider"), description: "How the model picker draws the thinking-effort control.", applies: NOW },
        Setting { key: "transcript.showThinking", kind: Kind::Switch, default: json!(true), description: "Show the agent's thinking blocks in chats.", applies: NOW },
        Setting { key: "transcript.autoExpandEdits", kind: Kind::Switch, default: json!(false), description: "Open file-edit activity in chats expanded instead of collapsed.", applies: NOW },
        Setting { key: "newChat.model", kind: Kind::Model, default: Value::Null, description: "The harness and model a new chat starts on, as {harness, model}, a bare model id, or null for Bridge's standard choice. Call settings_read with this key to list the models you can choose.", applies: NEW_CHATS },
        Setting { key: "newChat.effort", kind: Kind::Effort, default: Value::Null, description: "The thinking effort a new chat starts with: one of the default model's supported levels, or null for the provider default. Needs newChat.model set.", applies: NEW_CHATS },
        Setting { key: "newChat.createWorktree", kind: Kind::Switch, default: json!(false), description: "Start new project chats in their own isolated Git worktree.", applies: NEW_CHATS },
        Setting { key: "newChat.activeTurnInput", kind: Kind::Choice(&["steer", "queue"]), default: json!("steer"), description: "What a message sent while the agent is working does: steer joins the running turn, queue waits for it to finish.", applies: NEW_CHATS },
    ]
}

/// Every key a chat may change, for the tool's input schema.
pub fn keys() -> Vec<&'static str> {
    catalog().iter().map(|setting| setting.key).collect()
}

fn setting(key: &str) -> Result<Setting, ToolError> {
    catalog().into_iter().find(|setting| setting.key == key).ok_or_else(|| {
        let keys: Vec<&str> = catalog().iter().map(|setting| setting.key).collect();
        bad(format!("There is no setting \"{key}\" you can change. The settings are: {}. {NOT_HERE}", keys.join(", ")))
    })
}

fn enum_text<T: serde::Serialize>(value: &Option<T>) -> Value {
    value.as_ref().map(|value| serde_json::to_value(value).unwrap_or(Value::Null)).unwrap_or(Value::Null)
}

/// The stored value of one setting, `Null` when it was never set.
fn stored(key: &str, app: &AppPreferenceValues, chat: &ChatDefaults) -> Value {
    match key {
        "appearance.theme" => enum_text(&app.theme),
        "appearance.skin" => enum_text(&app.skin),
        "appearance.effortSelector" => enum_text(&app.effort_selector),
        "transcript.showThinking" => json!(app.show_thinking),
        "transcript.autoExpandEdits" => json!(app.auto_expand_edits),
        "newChat.model" => chat.model.as_ref().map_or(Value::Null, |model| json!({"harness": model.harness, "model": model.model})),
        "newChat.effort" => chat.effort.as_ref().map_or(Value::Null, |effort| json!(effort.value)),
        "newChat.createWorktree" => json!(chat.create_worktree),
        "newChat.activeTurnInput" => enum_text(&chat.active_turn_input),
        _ => Value::Null,
    }
}

fn usable(db: &rusqlite::Connection, descriptors: &[AdapterDescriptor]) -> Vec<AdapterDescriptor> {
    descriptors.iter().filter(|descriptor| descriptor.available && crate::agent_config::is_harness_enabled(db, &descriptor.id)).cloned().collect()
}

fn model_label(descriptors: &[AdapterDescriptor], harness: &str, model: Option<&str>) -> Option<String> {
    let descriptor = descriptors.iter().find(|descriptor| descriptor.id == harness)?;
    match model {
        Some(id) => descriptor.models.iter().find(|option| option.id == id).map(|option| format!("{} ({})", option.label, descriptor.label)),
        None => Some(format!("{} default model", descriptor.label)),
    }
}

fn read(core: &Arc<BridgeCore>, args: &Value) -> Result<Value, ToolError> {
    let key = args.get("key").and_then(Value::as_str).filter(|key| !key.is_empty());
    let search = args.get("search").and_then(Value::as_str).map(str::to_lowercase).filter(|text| !text.is_empty());
    let descriptors = core.adapter_registry.descriptors();
    let db = core.db.lock().unwrap();
    let app = crate::app_preferences::read(&db).map_err(service_error)?.values;
    let chat = crate::chat_defaults::view(&db, None).map_err(service_error)?.application.defaults;
    let effective = crate::chat_defaults::resolve(&db, &core.adapter_registry, &ResolveChatSettingsParams { workspace_id: None, overrides: ChatDefaults::default(), expected_revision: None }).ok();
    let describe = |setting: &Setting| {
        let value = stored(setting.key, &app, &chat);
        let mut entry = json!({
            "key": setting.key,
            "value": value,
            "default": setting.default,
            "description": setting.description,
            "appliesTo": setting.applies,
        });
        if let Kind::Choice(values) = setting.kind {
            entry["allowed"] = json!(values);
        }
        if let Kind::Switch = setting.kind {
            entry["allowed"] = json!([true, false]);
        }
        match (setting.key, &effective) {
            ("newChat.model", Some(resolved)) => {
                entry["effective"] = json!({"harness": resolved.model.harness, "model": resolved.model.model, "label": model_label(&descriptors, &resolved.model.harness, resolved.model.model.as_deref())});
            }
            ("newChat.effort", Some(resolved)) => entry["effective"] = json!(resolved.effort),
            _ => {}
        }
        entry
    };
    let Some(key) = key else {
        return Ok(json!({"settings": catalog().iter().map(describe).collect::<Vec<_>>(), "notHere": NOT_HERE}));
    };
    let setting = setting(key)?;
    let mut entry = describe(&setting);
    if matches!(setting.kind, Kind::Model | Kind::Effort) {
        let harnesses: Vec<Value> = usable(&db, &descriptors)
            .into_iter()
            .map(|descriptor| {
                let matching: Vec<Value> = descriptor
                    .models
                    .iter()
                    .filter(|option| option.available && option.compatible)
                    .filter(|option| search.as_ref().is_none_or(|text| format!("{} {} {}", descriptor.id, option.id, option.label).to_lowercase().contains(text)))
                    .map(|option| json!({"id": option.id, "label": option.label, "efforts": option.supported_effort_levels}))
                    .collect();
                let more = matching.len().saturating_sub(MODEL_LIST_CAP);
                let mut harness = json!({"harness": descriptor.id, "label": descriptor.label, "defaultModel": descriptor.default_model, "models": matching.into_iter().take(MODEL_LIST_CAP).collect::<Vec<_>>()});
                if more > 0 {
                    harness["moreModels"] = json!(format!("{more} more; pass search to narrow"));
                }
                harness
            })
            .filter(|harness| search.is_none() || harness["models"].as_array().is_some_and(|models| !models.is_empty()))
            .collect();
        entry["options"] = json!(harnesses);
    }
    Ok(entry)
}

fn service_error(error: crate::BridgeError) -> ToolError {
    ToolError { status: 400, message: error.to_string() }
}

fn switch(key: &str, value: &Value) -> Result<bool, ToolError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::String(text) if matches!(text.as_str(), "true" | "on") => Ok(true),
        Value::String(text) if matches!(text.as_str(), "false" | "off") => Ok(false),
        _ => Err(bad(format!("{key} takes true or false"))),
    }
}

fn choice<T: serde::de::DeserializeOwned>(key: &str, values: &[&str], value: &Value) -> Result<T, ToolError> {
    let text = value.as_str().map(str::to_lowercase).unwrap_or_default();
    serde_json::from_value(json!(text)).map_err(|_| bad(format!("{key} must be one of {}", values.join(", "))))
}

/// Turn a model value into a choice: `{harness, model}`, a harness id (its
/// default model), or a model id exactly one usable harness offers.
fn model_choice(value: &Value, usable: &[AdapterDescriptor]) -> Result<Option<ChatModelChoice>, ToolError> {
    let pick = |harness: &str, model: Option<String>| Some(ChatModelChoice { harness: harness.to_owned(), model, account: None });
    match value {
        Value::Null => Ok(None),
        Value::Object(object) => {
            let harness = object.get("harness").and_then(Value::as_str).ok_or_else(|| bad("newChat.model needs a harness, for example {\"harness\": \"claude\", \"model\": \"<id>\"}"))?;
            Ok(pick(harness, object.get("model").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned)))
        }
        Value::String(text) => {
            if usable.iter().any(|descriptor| descriptor.id == *text) {
                return Ok(pick(text, None));
            }
            // harness/model, when the prefix is a harness that offers the rest.
            // Model ids can contain slashes themselves (OpenCode's do), so an
            // unmatched prefix falls through to a plain id.
            if let Some((harness, model)) = text.split_once('/') {
                if usable.iter().any(|descriptor| descriptor.id == harness && descriptor.models.iter().any(|option| option.id == model)) {
                    return Ok(pick(harness, Some(model.to_owned())));
                }
            }
            let offering: Vec<&AdapterDescriptor> = usable.iter().filter(|descriptor| descriptor.models.iter().any(|option| option.id == *text && option.available && option.compatible)).collect();
            match offering.as_slice() {
                [one] => Ok(pick(&one.id, Some(text.clone()))),
                [] => Err(bad(format!("No available model has the id \"{text}\". Call settings_read with key newChat.model (and search) for the ids you can use."))),
                many => Err(bad(format!("Several harnesses offer \"{text}\" ({}); pass {{\"harness\": ..., \"model\": \"{text}\"}}.", many.iter().map(|descriptor| descriptor.id.as_str()).collect::<Vec<_>>().join(", ")))),
            }
        }
        _ => Err(bad("newChat.model takes {harness, model}, a model id, or null")),
    }
}

fn change_app(core: &Arc<BridgeCore>, setting: &Setting, value: &Value) -> Result<Value, ToolError> {
    let mut values = AppPreferenceValues::default();
    let allowed = match setting.kind {
        Kind::Choice(values) => values,
        _ => &[],
    };
    match setting.key {
        "appearance.theme" => values.theme = Some(choice::<ThemePreference>(setting.key, allowed, value)?),
        "appearance.skin" => values.skin = Some(choice::<ThemeSkin>(setting.key, allowed, value)?),
        "appearance.effortSelector" => values.effort_selector = Some(choice::<EffortSelectorStyle>(setting.key, allowed, value)?),
        "transcript.showThinking" => values.show_thinking = Some(switch(setting.key, value)?),
        "transcript.autoExpandEdits" => values.auto_expand_edits = Some(switch(setting.key, value)?),
        other => return Err(bad(format!("{other} is not an app preference"))),
    }
    let (previous, saved, changed) = {
        let db = core.db.lock().unwrap();
        let previous = crate::app_preferences::read(&db).map_err(service_error)?.values;
        let (saved, changed) = crate::app_preferences::save(&db, &values).map_err(service_error)?;
        (previous, saved.values, changed)
    };
    if changed {
        core.events.publish(crate::events::CoreEvent::AppPreferencesChanged);
    }
    let empty = ChatDefaults::default();
    let previous = stored(setting.key, &previous, &empty);
    Ok(json!({
        "key": setting.key,
        "previous": if previous.is_null() { setting.default.clone() } else { previous },
        "value": stored(setting.key, &saved, &empty),
        "changed": changed,
        "appliesTo": setting.applies,
    }))
}

fn change_new_chat(core: &Arc<BridgeCore>, setting: &Setting, value: &Value) -> Result<Value, ToolError> {
    let descriptors = core.adapter_registry.descriptors();
    let db = core.db.lock().unwrap();
    let record = crate::chat_defaults::view(&db, None).map_err(service_error)?.application;
    let mut defaults = record.defaults.clone();
    let mut notes: Vec<String> = Vec::new();
    let field = match setting.key {
        "newChat.model" => {
            let mut choice = model_choice(value, &usable(&db, &descriptors))?;
            // Keep the account the person picked when the harness stays the same.
            if let (Some(next), Some(current)) = (choice.as_mut(), defaults.model.as_ref()) {
                if next.harness == current.harness {
                    next.account = current.account.clone();
                }
            }
            // A stored effort the new model cannot run would make the save
            // fail; drop it and say so rather than refusing the model.
            if let (Some(effort), Some(next)) = (defaults.effort.as_ref().and_then(|effort| effort.value.clone()), choice.as_ref()) {
                let supported = descriptors
                    .iter()
                    .find(|descriptor| descriptor.id == next.harness)
                    .and_then(|descriptor| next.model.as_ref().and_then(|id| descriptor.models.iter().find(|option| option.id == *id)))
                    .is_some_and(|option| option.supported_effort_levels.iter().any(|level| *level == effort));
                if !supported {
                    defaults.effort = None;
                    notes.push(format!("The default thinking effort ({effort}) does not apply to this model, so it was reset to the provider default."));
                }
            }
            if choice.is_none() {
                defaults.effort = None;
            }
            defaults.model = choice;
            "model"
        }
        "newChat.effort" => {
            defaults.effort = match value {
                Value::Null => None,
                Value::String(level) => Some(ChatEffortChoice { value: Some(level.to_lowercase()) }),
                _ => return Err(bad("newChat.effort takes a level name, or null for the provider default")),
            };
            if defaults.effort.is_some() && defaults.model.is_none() {
                return Err(bad("Set newChat.model first: a thinking effort belongs to a model."));
            }
            "effort"
        }
        "newChat.createWorktree" => {
            defaults.create_worktree = if value.is_null() { None } else { Some(switch(setting.key, value)?) };
            "createWorktree"
        }
        "newChat.activeTurnInput" => {
            defaults.active_turn_input = if value.is_null() {
                None
            } else {
                let allowed = match setting.kind {
                    Kind::Choice(values) => values,
                    _ => &[],
                };
                Some(choice::<ActiveTurnInput>(setting.key, allowed, value)?)
            };
            "activeTurnInput"
        }
        other => return Err(bad(format!("{other} is not a new-chat default"))),
    };
    let previous = stored(setting.key, &AppPreferenceValues::default(), &record.defaults);
    let saved = crate::chat_defaults::save(&db, &descriptors, &SaveChatDefaultsParams { project_id: None, expected_revision: record.revision, defaults }).map_err(service_error)?;
    let overriding = crate::chat_defaults::projects_overriding(&db, field).map_err(service_error)?;
    drop(db);
    core.events.publish(crate::events::CoreEvent::StateChanged);
    if !overriding.is_empty() {
        notes.push(format!("These projects set their own value and keep it: {}.", overriding.join(", ")));
    }
    let value = stored(setting.key, &AppPreferenceValues::default(), &saved.application.defaults);
    let mut result = json!({"key": setting.key, "previous": previous, "value": value, "changed": previous != value, "appliesTo": setting.applies});
    if setting.key == "newChat.model" {
        if let Some(model) = &saved.application.defaults.model {
            result["label"] = json!(model_label(&descriptors, &model.harness, model.model.as_deref()));
        }
    }
    if !notes.is_empty() {
        result["notes"] = json!(notes);
    }
    Ok(result)
}

fn change(core: &Arc<BridgeCore>, args: &Value) -> Result<Value, ToolError> {
    let key = args.get("key").and_then(Value::as_str).ok_or_else(|| bad(format!("key and value are required, for example {{\"key\": \"appearance.theme\", \"value\": \"light\"}}. The keys are: {}.", keys().join(", "))))?;
    let setting = setting(key)?;
    // The schema asks for text so every harness can send it; "default" (or
    // null) resets, which for a new-chat default means inherit.
    let value = match args.get("value").cloned().unwrap_or(Value::Null) {
        Value::String(text) if matches!(text.trim().to_lowercase().as_str(), "default" | "null" | "reset" | "") => Value::Null,
        other => other,
    };
    if key.starts_with("newChat.") {
        change_new_chat(core, &setting, &value)
    } else {
        let value = if value.is_null() { setting.default.clone() } else { value };
        change_app(core, &setting, &value)
    }
}

/// Run one settings tool. `top_level` is the chat's access: only a top-level
/// chat may read or change Bridge's settings.
pub fn run(core: &Arc<BridgeCore>, top_level: bool, tool: &str, args: &Value) -> Result<Value, ToolError> {
    if !top_level {
        return Err(ToolError { status: 403, message: "Only a top-level chat changes Bridge's settings; tell your orchestrator instead.".into() });
    }
    match tool {
        READ_TOOL => read(core, args),
        CHANGE_TOOL => change(core, args),
        other => Err(ToolError { status: 404, message: format!("unknown settings tool {other}") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        (dir, core)
    }

    #[test]
    fn a_worker_can_neither_read_nor_change() {
        let (_dir, core) = core();
        for tool in TOOLS {
            let refused = run(&core, false, tool, &json!({"key": "appearance.theme", "value": "light"})).unwrap_err();
            assert_eq!(refused.status, 403);
        }
        assert_eq!(crate::app_preferences::read(&core.db.lock().unwrap()).unwrap().revision, 0);
    }

    #[test]
    fn read_lists_every_setting_with_value_default_and_what_is_not_here() {
        let (_dir, core) = core();
        let listing = run(&core, true, READ_TOOL, &json!({})).unwrap();
        let keys: Vec<&str> = listing["settings"].as_array().unwrap().iter().map(|entry| entry["key"].as_str().unwrap()).collect();
        assert_eq!(keys, catalog().iter().map(|setting| setting.key).collect::<Vec<_>>());
        let theme = &listing["settings"][0];
        assert_eq!((theme["value"].clone(), theme["default"].clone(), theme["allowed"].clone()), (Value::Null, json!("system"), json!(["system", "light", "dark"])));
        assert!(listing["notHere"].as_str().unwrap().contains("API keys"));
        assert!(!keys.iter().any(|key| key.contains("access")), "access is never in the catalog");
    }

    #[test]
    fn changing_an_app_preference_saves_it_and_publishes_one_hint() {
        let (_dir, core) = core();
        let mut events = core.events.subscribe();
        let changed = run(&core, true, CHANGE_TOOL, &json!({"key": "appearance.theme", "value": "Light"})).unwrap();
        assert_eq!((changed["previous"].clone(), changed["value"].clone(), changed["changed"].clone()), (json!("system"), json!("light"), json!(true)));
        assert!(matches!(events.try_recv(), Ok(crate::events::CoreEvent::AppPreferencesChanged)));
        let hidden = run(&core, true, CHANGE_TOOL, &json!({"key": "transcript.showThinking", "value": false})).unwrap();
        assert_eq!(hidden["value"], false);
        let stored = crate::app_preferences::read(&core.db.lock().unwrap()).unwrap();
        assert_eq!(stored.values.theme, Some(ThemePreference::Light));
        assert_eq!(stored.values.show_thinking, Some(false));
        // The same value again is not a change and sends no hint.
        let _ = events.try_recv();
        let again = run(&core, true, CHANGE_TOOL, &json!({"key": "appearance.theme", "value": "light"})).unwrap();
        assert_eq!(again["changed"], false);
        assert!(events.try_recv().is_err());
        assert_eq!(run(&core, true, READ_TOOL, &json!({})).unwrap()["settings"][0]["value"], "light");
    }

    #[test]
    fn bad_keys_and_values_change_nothing_and_say_what_is_valid() {
        let (_dir, core) = core();
        let wrong_value = run(&core, true, CHANGE_TOOL, &json!({"key": "appearance.theme", "value": "purple"})).unwrap_err();
        assert!(wrong_value.message.contains("system, light, dark"), "{}", wrong_value.message);
        let wrong_type = run(&core, true, CHANGE_TOOL, &json!({"key": "transcript.showThinking", "value": 3})).unwrap_err();
        assert!(wrong_type.message.contains("true or false"));
        let missing = run(&core, true, CHANGE_TOOL, &json!({"theme": "light"})).unwrap_err();
        assert!(missing.message.contains("appearance.theme") && missing.message.contains("newChat.model"), "{}", missing.message);
        let unknown = run(&core, true, CHANGE_TOOL, &json!({"key": "appearance.font", "value": "big"})).unwrap_err();
        assert!(unknown.message.contains("appearance.theme"), "lists the valid keys: {}", unknown.message);
        for sensitive in ["newChat.access", "permissions.fullAccess", "keys.openai"] {
            let refused = run(&core, true, CHANGE_TOOL, &json!({"key": sensitive, "value": "full"})).unwrap_err();
            assert!(refused.message.contains("Bridge Settings"), "{sensitive}: {}", refused.message);
        }
        let db = core.db.lock().unwrap();
        assert_eq!(crate::app_preferences::read(&db).unwrap().revision, 0);
        assert_eq!(crate::chat_defaults::view(&db, None).unwrap().application.revision, 0);
    }
    struct Stub(&'static str);
    impl crate::adapters::HarnessAdapter for Stub {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            let models = if self.0 == "claude" {
                json!([{"id": "opus", "label": "Opus", "tier": "strong", "defaultForTier": true, "supportedEffortLevels": ["low", "high", "max"]}, {"id": "shared", "label": "Shared", "tier": "standard", "defaultForTier": true, "supportedEffortLevels": []}])
            } else {
                json!([{"id": "standard", "label": "Standard", "tier": "standard", "defaultForTier": true, "supportedEffortLevels": ["low", "medium", "high", "xhigh"]}, {"id": "shared", "label": "Shared", "tier": "fast", "defaultForTier": true, "supportedEffortLevels": []}])
            };
            serde_json::from_value(json!({"id": self.0, "label": if self.0 == "claude" { "Claude" } else { "Codex" }, "available": true, "authState": "unknown", "capabilities": [], "sandboxModes": ["read_only", "workspace_write", "danger_full_access"], "defaultModel": null, "models": models})).unwrap()
        }
        fn start(&self, _: crate::adapters::StartRequest<'_>) -> Result<crate::adapters::StartedAdapter, crate::BridgeError> {
            panic!("settings must not start a provider")
        }
        fn resume(&self, _: crate::adapters::ResumeRequest<'_>) -> Result<crate::adapters::StartedAdapter, crate::BridgeError> {
            panic!("settings must not resume a provider")
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _: &Value) -> Vec<crate::agent::NormalizedEvent> {
            vec![]
        }
    }

    fn with_models() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(dir.path());
        let mut registry = crate::adapters::AdapterRegistry::empty();
        registry.register(Box::new(Stub("codex"))).unwrap();
        registry.register(Box::new(Stub("claude"))).unwrap();
        core.adapter_registry = Arc::new(registry);
        (dir, Arc::new(core))
    }

    fn change(core: &Arc<BridgeCore>, key: &str, value: Value) -> Result<Value, ToolError> {
        run(core, true, CHANGE_TOOL, &json!({"key": key, "value": value}))
    }

    #[test]
    fn the_model_list_names_usable_models_with_their_efforts_and_narrows_by_search() {
        let (_dir, core) = with_models();
        let listed = run(&core, true, READ_TOOL, &json!({"key": "newChat.model"})).unwrap();
        let harnesses: Vec<&str> = listed["options"].as_array().unwrap().iter().map(|entry| entry["harness"].as_str().unwrap()).collect();
        assert_eq!(harnesses, ["claude", "codex"]);
        assert_eq!(listed["options"][0]["models"][0], json!({"id": "opus", "label": "Opus", "efforts": ["low", "high", "max"]}));
        let narrowed = run(&core, true, READ_TOOL, &json!({"key": "newChat.model", "search": "OPUS"})).unwrap();
        assert_eq!(narrowed["options"].as_array().unwrap().len(), 1, "a harness with no match is left out");
        assert_eq!(narrowed["options"][0]["models"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_default_model_takes_an_object_a_unique_id_or_a_harness_and_refuses_the_rest() {
        let (_dir, core) = with_models();
        let mut events = core.events.subscribe();
        let set = change(&core, "newChat.model", json!("opus")).unwrap();
        assert_eq!(set["value"], json!({"harness": "claude", "model": "opus"}));
        assert_eq!(set["label"], "Opus (Claude)");
        assert!(matches!(events.try_recv(), Ok(crate::events::CoreEvent::StateChanged)));

        let ambiguous = change(&core, "newChat.model", json!("shared")).unwrap_err();
        assert!(ambiguous.message.contains("claude, codex"), "{}", ambiguous.message);
        let missing = change(&core, "newChat.model", json!("gpt-9")).unwrap_err();
        assert!(missing.message.contains("settings_read"), "{}", missing.message);
        let unknown = change(&core, "newChat.model", json!({"harness": "codex", "model": "nope"})).unwrap_err();
        assert!(unknown.message.contains("unavailable"), "{}", unknown.message);

        assert_eq!(change(&core, "newChat.model", json!({"harness": "codex", "model": "shared"})).unwrap()["value"], json!({"harness": "codex", "model": "shared"}));
        assert_eq!(change(&core, "newChat.model", json!("codex")).unwrap()["value"], json!({"harness": "codex", "model": null}));
        assert_eq!(change(&core, "newChat.model", Value::Null).unwrap()["value"], Value::Null);
    }

    #[test]
    fn a_new_model_drops_an_effort_it_cannot_run_and_keeps_every_other_field() {
        let (_dir, core) = with_models();
        // Access and target are not in the catalog, but a change must keep them.
        {
            let db = core.db.lock().unwrap();
            let defaults = ChatDefaults { access: Some(bridge_protocol::messages::ChatAccess::Full), target: Some(bridge_protocol::messages::ChatLaunchTarget::Direct), ..Default::default() };
            crate::chat_defaults::save(&db, &core.adapter_registry.descriptors(), &SaveChatDefaultsParams { project_id: None, expected_revision: 0, defaults }).unwrap();
        }
        let refused = change(&core, "newChat.effort", json!("high")).unwrap_err();
        assert!(refused.message.contains("newChat.model first"), "{}", refused.message);
        change(&core, "newChat.model", json!("opus")).unwrap();
        assert_eq!(change(&core, "newChat.effort", json!("max")).unwrap()["value"], "max");
        let unsupported = change(&core, "newChat.effort", json!("xhigh")).unwrap_err();
        assert!(unsupported.message.contains("does not support"), "{}", unsupported.message);

        let switched = change(&core, "newChat.model", json!("standard")).unwrap();
        assert!(switched["notes"][0].as_str().unwrap().contains("(max)"), "{switched}");
        change(&core, "newChat.createWorktree", json!(true)).unwrap();
        change(&core, "newChat.activeTurnInput", json!("queue")).unwrap();

        let stored = crate::chat_defaults::view(&core.db.lock().unwrap(), None).unwrap().application.defaults;
        assert_eq!(stored.effort, None);
        assert_eq!(stored.access, Some(bridge_protocol::messages::ChatAccess::Full));
        assert_eq!(stored.target, Some(bridge_protocol::messages::ChatLaunchTarget::Direct));
        assert_eq!(stored.create_worktree, Some(true));
        assert_eq!(stored.active_turn_input, Some(ActiveTurnInput::Queue));
    }

    #[test]
    fn a_project_with_its_own_model_is_named_in_the_result() {
        let (dir, core) = with_models();
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Website',?1,'now')", [dir.path().join("p").to_string_lossy()]).unwrap();
            let defaults = ChatDefaults { model: Some(ChatModelChoice { harness: "codex".into(), model: Some("standard".into()), account: None }), ..Default::default() };
            crate::chat_defaults::save(&db, &core.adapter_registry.descriptors(), &SaveChatDefaultsParams { project_id: Some("p".into()), expected_revision: 0, defaults }).unwrap();
        }
        let set = change(&core, "newChat.model", json!("opus")).unwrap();
        assert!(set["notes"][0].as_str().unwrap().contains("Website"), "{set}");
        let worktree = change(&core, "newChat.createWorktree", json!(true)).unwrap();
        assert!(worktree.get("notes").is_none(), "the project does not set this field");
    }
}
