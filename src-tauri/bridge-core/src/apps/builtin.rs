//! The built-in apps: every screen that used to own a sidebar row, plus
//! Outgoing, which has no screen and shows up as a card in the chat.
//!
//! The order is the launcher's. The frontend keeps only what a client needs
//! per id (icon, view, keywords); everything else comes from here.

use std::sync::OnceLock;

use bridge_protocol::messages::{AppManifest, AppSlot, AppSource, AppTool, AppView};

/// Where an app's badge count comes from. Wired here, never read from a
/// manifest, so a community app cannot claim another app's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeSource {
    None,
    /// Runs waiting for approval, and finished runs not yet read.
    WorkflowRuns,
    /// Memory proposals waiting for review in the account scope.
    MemoryProposals,
    /// Trails in the needs-you state.
    TrailsNeedingYou,
}

#[derive(Debug, Clone)]
pub struct BuiltinApp {
    pub manifest: AppManifest,
    pub badge: BadgeSource,
    /// False for the Store: it is where apps are turned back on.
    pub can_disable: bool,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    slots: &'static [AppSlot],
    tools: &'static [(&'static str, bool)],
    events: &'static [&'static str],
    memory: &'static [&'static str],
    badge: BadgeSource,
    can_disable: bool,
}

const SCREEN: &[AppSlot] = &[AppSlot::Screen];
const SCREEN_DOCK: &[AppSlot] = &[AppSlot::Screen, AppSlot::Dock];

const SPECS: &[Spec] = &[
    Spec {
        id: "notes",
        name: "Notes",
        description: "Pages, tasks and quick captures",
        slots: SCREEN,
        tools: &[
            ("notes_search", false),
            ("notes_read", false),
            ("notes_create", true),
            ("notes_edit", true),
            ("notes_append", true),
            ("notes_set", true),
            ("notes_draw", true),
        ],
        events: &[],
        memory: &[],
        badge: BadgeSource::None,
        can_disable: true,
    },
    Spec { id: "trails", name: "Trails", description: "One page per piece of work: its chats, notes, PRs and drafts", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::TrailsNeedingYou, can_disable: true },
    Spec { id: "pulls", name: "Pulls", description: "Pull requests across your repositories", slots: SCREEN_DOCK, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "workflows", name: "Workflows", description: "Runs that repeat on a schedule", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::WorkflowRuns, can_disable: true },
    Spec {
        id: "memory",
        name: "Memory",
        description: "What Bridge remembers about you",
        slots: SCREEN,
        tools: &[("memory_search", false), ("memory_propose", false)],
        events: &[],
        memory: &[],
        badge: BadgeSource::MemoryProposals,
        can_disable: true,
    },
    Spec { id: "mission-control", name: "Mission Control", description: "Every agent on one board", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "terminals", name: "Terminals", description: "A shell for each stream of work", slots: SCREEN_DOCK, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "usage", name: "Usage", description: "Spend and limits per provider", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "projects", name: "Projects", description: "Your repositories and folders", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "agents", name: "Agents", description: "Your own agents and Bridge's built-in ones", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: true },
    Spec { id: "store", name: "Store", description: "Apps, plugins and skills", slots: SCREEN, tools: &[], events: &[], memory: &[], badge: BadgeSource::None, can_disable: false },
    Spec {
        id: "outgoing",
        name: "Outgoing",
        description: "Messages a chat drafts for you to send",
        slots: &[AppSlot::Card],
        tools: &[("outgoing_draft", false)],
        events: &["outgoing.sent"],
        memory: &["the people, channels and repositories the person writes to"],
        badge: BadgeSource::None,
        can_disable: true,
    },
];

fn build(spec: &Spec) -> BuiltinApp {
    BuiltinApp {
        manifest: AppManifest {
            id: spec.id.into(),
            name: spec.name.into(),
            description: spec.description.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            source: AppSource::Builtin,
            views: spec.slots.iter().map(|slot| AppView { slot: *slot, title: None }).collect(),
            tools: spec.tools.iter().map(|(name, asks_first)| AppTool { name: (*name).into(), asks_first: *asks_first }).collect(),
            events: spec.events.iter().map(|event| (*event).into()).collect(),
            memory: spec.memory.iter().map(|line| (*line).into()).collect(),
        },
        badge: spec.badge,
        can_disable: spec.can_disable,
    }
}

/// Every built-in, validated once. A built-in that breaks a manifest rule is
/// a bug in this file, so it fails loud.
pub fn builtins() -> &'static [BuiltinApp] {
    static APPS: OnceLock<Vec<BuiltinApp>> = OnceLock::new();
    APPS.get_or_init(|| {
        let apps: Vec<BuiltinApp> = SPECS.iter().map(build).collect();
        let reserved: Vec<&str> = super::CORE_TOOLS.iter().map(|(name, _)| *name).collect();
        if let Err(error) = super::manifest::validate_set(apps.iter().map(|app| &app.manifest), &reserved) {
            panic!("built-in app manifests are invalid: {error}");
        }
        apps
    })
}

pub fn builtin(id: &str) -> Option<&'static BuiltinApp> {
    builtins().iter().find(|app| app.manifest.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn builtins_validate_and_list_twelve_apps_in_order() {
        let ids: Vec<&str> = builtins().iter().map(|app| app.manifest.id.as_str()).collect();
        assert_eq!(
            ids,
            ["notes", "trails", "pulls", "workflows", "memory", "mission-control", "terminals", "usage", "projects", "agents", "store", "outgoing"]
        );
        assert_eq!(builtins().iter().filter(|app| !app.can_disable).map(|app| app.manifest.id.as_str()).collect::<Vec<_>>(), ["store"]);
        assert_eq!(builtin("outgoing").unwrap().manifest.views, vec![AppView { slot: AppSlot::Card, title: None }]);
    }

    #[test]
    fn every_bridge_tool_is_owned_by_one_app_or_core() {
        let mut listed: Vec<String> = crate::mcp_apps::notes_tools::all_definitions()
            .iter()
            .map(|definition| definition["name"].as_str().unwrap().to_owned())
            .collect();
        listed.push(crate::mcp_apps::tool::TOOL_NAME.to_owned());
        for tool in &listed {
            let core = super::super::CORE_TOOLS.iter().any(|(name, _)| name == tool);
            let owners = builtins().iter().filter(|app| app.manifest.tools.iter().any(|owned| &owned.name == tool)).count();
            assert!(core ^ (owners == 1), "{tool} must belong to exactly one app or be a core tool (core {core}, owners {owners})");
        }
        // And nothing claims a tool the server cannot list.
        for app in builtins() {
            for tool in &app.manifest.tools {
                assert!(listed.contains(&tool.name), "{} declares {} but the server has no such tool", app.manifest.id, tool.name);
            }
        }
    }

    /// Every `id: "..."` in a TypeScript source, in order.
    fn ts_ids(source: &str) -> BTreeSet<String> {
        source
            .split("id: \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_owned)
            .collect()
    }

    /// A client draws only the apps it has a descriptor for, so an app core
    /// lists that the frontend does not know would vanish without an error.
    /// Core, the frontend descriptors and the mock must name the same apps.
    #[test]
    fn frontend_and_mock_list_exactly_the_builtin_ids() {
        let core: BTreeSet<String> = builtins().iter().map(|app| app.manifest.id.clone()).collect();
        let frontend = ts_ids(include_str!("../../../../src/apps/registry.ts"));
        let mock = ts_ids(include_str!("../../../../src/appsMock.ts"));
        assert_eq!(frontend, core, "src/apps/registry.ts (BUILTIN_APPS plus CARD_APPS) must describe exactly core's apps");
        assert_eq!(mock, core, "src/appsMock.ts must serve exactly core's apps");
    }

    /// The Claude sidecar runs exactly the tools that do not ask first and
    /// refuses the rest outside Full access. This keeps the two lists one.
    #[test]
    fn sidecar_allowlist_matches_asks_first() {
        let source = include_str!("../../../../sidecar/claude-agent/options.mjs");
        let mut allowed = BTreeSet::new();
        for list in ["BRIDGE_READ_TOOLS", "BRIDGE_REVIEWED_TOOLS", "BRIDGE_PREFERENCE_TOOLS"] {
            let start = source.find(&format!("export const {list} = [")).unwrap_or_else(|| panic!("{list} not found"));
            let body = &source[start..];
            let body = &body[body.find('[').unwrap() + 1..body.find(']').unwrap()];
            for item in body.split(',') {
                let name = item.trim().trim_matches('"');
                if !name.is_empty() {
                    allowed.insert(name.strip_prefix("mcp__bridge__").expect("a bridge tool").to_owned());
                }
            }
        }
        let mut quiet: BTreeSet<String> = builtins()
            .iter()
            .flat_map(|app| app.manifest.tools.iter())
            .filter(|tool| !tool.asks_first)
            .map(|tool| tool.name.clone())
            .collect();
        quiet.extend(super::super::CORE_TOOLS.iter().filter(|(_, asks)| !asks).map(|(name, _)| (*name).to_owned()));
        assert_eq!(allowed, quiet);
    }
}
