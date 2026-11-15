//! Apps: the features Bridge shows through the launcher, the dock, the sidebar
//! pins and chat cards, each described by a manifest that core owns.
//!
//! A manifest says only what any app may declare: where it shows up, the
//! agent tools it brings, the events it records and what memory may keep from
//! them. Privileges a built-in has (its badge source, whether it can be
//! turned off, its prompt text) are wired in core, never read from a
//! manifest, so a community manifest cannot claim them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most apps a person can pin to the sidebar.
pub const MAX_APP_PINS: usize = 3;

/// Where an app came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AppSource {
    /// Ships with Bridge.
    Builtin,
    /// Installed by the person.
    Community,
    /// A source a newer daemon knows and this client does not.
    #[serde(other)]
    Other,
}

/// A place in Bridge an app can show up besides the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AppSlot {
    /// A full screen of its own.
    Screen,
    /// A pane in a chat's dock.
    Dock,
    /// A card inside the conversation.
    Card,
    /// A command in the launcher.
    Launcher,
    /// A slot a newer daemon knows and this client does not. Core refuses it
    /// in a manifest.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub slot: AppSlot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// One agent tool an app brings, as named on the `bridge` MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppTool {
    pub name: String,
    /// The tool changes something outside the chat, so it runs only when the
    /// person allows it. A manifest can make a tool ask; it can never make a
    /// tool skip an approval that policy requires.
    pub asks_first: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppManifest {
    /// Kebab-case, at most 40 characters, unique among apps.
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: AppSource,
    pub views: Vec<AppView>,
    pub tools: Vec<AppTool>,
    /// Event types the app records, each prefixed with its namespace.
    pub events: Vec<String>,
    /// Plain-language lines that limit what memory may keep from the app's
    /// events. Empty means memory never reads them.
    pub memory: Vec<String>,
}

/// Something waiting on the person in an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppBadge {
    pub count: u32,
}

/// An app as every client sees it: its manifest and its state on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    pub manifest: AppManifest,
    pub enabled: bool,
    /// False for the apps Bridge needs to manage the others (the Store).
    pub can_disable: bool,
    /// The sidebar position, from 0, or null when the app is not pinned.
    pub pinned: Option<u32>,
    /// Null when the app has no badge, is off, or its count could not be read.
    pub badge: Option<AppBadge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListAppsResult {
    pub apps: Vec<AppSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAppPinnedParams {
    pub id: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAppEnabledParams {
    pub id: String,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(value: T) {
        let text = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<T>(&text).unwrap(), value);
    }

    fn manifest() -> AppManifest {
        AppManifest {
            id: "outgoing".into(),
            name: "Outgoing".into(),
            description: "Messages a chat drafts for you".into(),
            version: "1.0.0".into(),
            source: AppSource::Builtin,
            views: vec![AppView { slot: AppSlot::Card, title: None }],
            tools: vec![AppTool { name: "outgoing_draft".into(), asks_first: false }],
            events: vec!["outgoing.sent".into()],
            memory: vec!["the people and channels the person writes to".into()],
        }
    }

    #[test]
    fn every_apps_type_round_trips() {
        round_trip(manifest());
        let summary = AppSummary { manifest: manifest(), enabled: true, can_disable: true, pinned: Some(0), badge: Some(AppBadge { count: 2 }) };
        round_trip(summary.clone());
        round_trip(ListAppsResult { apps: vec![summary] });
        round_trip(SetAppPinnedParams { id: "notes".into(), pinned: true });
        round_trip(SetAppEnabledParams { id: "notes".into(), enabled: false });
    }

    #[test]
    fn wire_names_are_camel_case() {
        let summary = AppSummary { manifest: manifest(), enabled: false, can_disable: true, pinned: None, badge: None };
        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["canDisable"], true);
        assert_eq!(value["pinned"], serde_json::Value::Null);
        assert_eq!(value["manifest"]["tools"][0]["asksFirst"], false);
        assert_eq!(value["manifest"]["views"][0], json!({"slot": "card"}));
    }

    #[test]
    fn unknown_sources_and_slots_decode_as_other() {
        assert_eq!(serde_json::from_value::<AppSource>(json!("signed")).unwrap(), AppSource::Other);
        assert_eq!(serde_json::from_value::<AppSlot>(json!("statusBar")).unwrap(), AppSlot::Other);
    }

    #[test]
    fn params_refuse_unknown_fields() {
        assert!(serde_json::from_value::<SetAppPinnedParams>(json!({"id": "notes", "pinned": true, "x": 1})).is_err());
        assert!(serde_json::from_value::<SetAppEnabledParams>(json!({"id": "notes", "enabled": true, "x": 1})).is_err());
    }
}
