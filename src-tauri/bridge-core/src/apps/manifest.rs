//! Manifest validation: the rules every app, built-in or not, must meet.
//!
//! Conflicts fail here, when the registry is built, never when a tool is
//! called: an unknown slot, a tool listed twice, a tool two apps claim.

use std::collections::BTreeMap;

use bridge_protocol::messages::{AppManifest, AppSlot};

pub const MAX_ID_CHARS: usize = 40;
pub const MAX_MEMORY_LINES: usize = 8;
pub const MAX_MEMORY_LINE_CHARS: usize = 200;

fn is_kebab(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && !text.ends_with('-')
        && !text.contains("--")
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The namespace an app's event types start with: its id without dashes, so
/// `mission-control` records `missioncontrol.*`.
pub fn event_namespace(id: &str) -> String {
    id.replace('-', "")
}

/// Check one manifest on its own. The message names the app and the rule.
pub fn validate(manifest: &AppManifest) -> Result<(), String> {
    let id = manifest.id.as_str();
    if id.chars().count() > MAX_ID_CHARS || !is_kebab(id) {
        return Err(format!("app id \"{id}\" must be kebab-case and at most {MAX_ID_CHARS} characters"));
    }
    if manifest.name.trim().is_empty() {
        return Err(format!("app {id} needs a name"));
    }
    for view in &manifest.views {
        if view.slot == AppSlot::Other {
            return Err(format!("app {id} declares a slot Bridge does not know"));
        }
    }
    let mut seen = Vec::new();
    for tool in &manifest.tools {
        if tool.name.trim().is_empty() {
            return Err(format!("app {id} declares a tool without a name"));
        }
        if seen.contains(&tool.name.as_str()) {
            return Err(format!("app {id} declares the tool {} twice", tool.name));
        }
        seen.push(tool.name.as_str());
    }
    let prefix = format!("{}.", event_namespace(id));
    for event in &manifest.events {
        if !event.starts_with(&prefix) || event.len() == prefix.len() {
            return Err(format!("app {id} event \"{event}\" must start with \"{prefix}\""));
        }
    }
    if manifest.memory.len() > MAX_MEMORY_LINES {
        return Err(format!("app {id} has more than {MAX_MEMORY_LINES} memory lines"));
    }
    for line in &manifest.memory {
        if line.trim().is_empty() || line.chars().count() > MAX_MEMORY_LINE_CHARS {
            return Err(format!("app {id} memory lines must be 1 to {MAX_MEMORY_LINE_CHARS} characters"));
        }
    }
    Ok(())
}

/// Check a set of manifests together: each on its own, then that ids are
/// unique, that no tool is claimed by two apps, and that no app claims a
/// name in `reserved` (the core tools).
pub fn validate_set<'a>(manifests: impl IntoIterator<Item = &'a AppManifest>, reserved: &[&str]) -> Result<(), String> {
    let mut ids = Vec::new();
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for manifest in manifests {
        validate(manifest)?;
        if ids.contains(&manifest.id.as_str()) {
            return Err(format!("two apps use the id {}", manifest.id));
        }
        ids.push(manifest.id.as_str());
        for tool in &manifest.tools {
            if reserved.contains(&tool.name.as_str()) {
                return Err(format!("app {} claims the core tool {}", manifest.id, tool.name));
            }
            if let Some(other) = owners.insert(tool.name.as_str(), manifest.id.as_str()) {
                return Err(format!("the tool {} is claimed by both {other} and {}", tool.name, manifest.id));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::messages::{AppSource, AppTool, AppView};

    fn app(id: &str, tools: &[&str]) -> AppManifest {
        AppManifest {
            id: id.into(),
            name: "App".into(),
            description: String::new(),
            version: "1.0.0".into(),
            source: AppSource::Builtin,
            views: vec![AppView { slot: AppSlot::Screen, title: None }],
            tools: tools.iter().map(|name| AppTool { name: (*name).into(), asks_first: false }).collect(),
            events: Vec::new(),
            memory: Vec::new(),
        }
    }

    #[test]
    fn unknown_slot_is_rejected() {
        let mut manifest = app("linear", &[]);
        manifest.views.push(AppView { slot: AppSlot::Other, title: None });
        assert!(validate(&manifest).unwrap_err().contains("slot"));
    }

    #[test]
    fn duplicate_tool_in_one_app_is_rejected() {
        assert!(validate(&app("linear", &["linear_search", "linear_search"])).unwrap_err().contains("twice"));
    }

    #[test]
    fn a_tool_owned_by_two_apps_is_rejected() {
        let error = validate_set([&app("linear", &["search"]), &app("jira", &["search"])], &[]).unwrap_err();
        assert!(error.contains("both linear and jira"), "{error}");
        let error = validate_set([&app("linear", &["visualize"])], &["visualize"]).unwrap_err();
        assert!(error.contains("core tool"), "{error}");
        assert!(validate_set([&app("linear", &[]), &app("linear", &[])], &[]).unwrap_err().contains("id"));
    }

    #[test]
    fn bad_ids_events_and_memory_lines_are_rejected() {
        for id in ["", "Linear", "linear_triage", "-linear", "linear-", "a--b", &"a".repeat(41)] {
            assert!(validate(&app(id, &[])).is_err(), "{id:?} is not a valid id");
        }
        assert!(validate(&app("linear-triage", &[])).is_ok());
        let mut manifest = app("linear-triage", &[]);
        manifest.events = vec!["linear.issue.opened".into()];
        assert!(validate(&manifest).unwrap_err().contains("lineartriage."));
        manifest.events = vec!["lineartriage.issue.opened".into()];
        assert!(validate(&manifest).is_ok());
        manifest.memory = vec!["x".repeat(201)];
        assert!(validate(&manifest).is_err());
        manifest.memory = vec!["team names".into(); 9];
        assert!(validate(&manifest).is_err());
        manifest.memory = vec!["team names".into()];
        assert!(validate(&manifest).is_ok());
    }
}
