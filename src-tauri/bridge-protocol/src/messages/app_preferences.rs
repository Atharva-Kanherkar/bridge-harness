//! Appearance and transcript choices the app draws itself. The core keeps them
//! so a chat, a second window or the web client can change them; each window
//! keeps a copy in localStorage only to paint the first frame.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSkin {
    Graphite,
    Vibrancy,
    Omarchy,
}

/// Absent means never set: the app keeps its built-in default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AppPreferenceValues {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemePreference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<ThemeSkin>,
    /// Retired: the model picker's effort style (slider, sentence or list).
    /// Bridge draws one effort control now. The key is still read, and
    /// dropped, so a record or a client from an earlier build parses rather
    /// than resetting every other preference; it is never written.
    #[serde(skip_serializing, deserialize_with = "drop_retired")]
    #[schemars(skip)]
    pub effort_selector: (),
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_expand_edits: Option<bool>,
    /// Let personal apps call servers connected only in OpenCode, through its
    /// gated courier. Off unless the person turns it on: OpenCode's code mode
    /// can make network requests Bridge cannot approve. Never changeable by a
    /// chat's `settings_change`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_opencode_app_courier: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPreferences {
    pub revision: u32,
    pub values: AppPreferenceValues,
}

/// Merge: only the keys present change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAppPreferencesParams {
    pub values: AppPreferenceValues,
}

fn drop_retired<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_are_lowercase_and_unset_keys_stay_off_the_wire() {
        let values = AppPreferenceValues { theme: Some(ThemePreference::Light), show_thinking: Some(false), ..Default::default() };
        assert_eq!(serde_json::to_value(&values).unwrap(), json!({"theme": "light", "showThinking": false}));
        let parsed: AppPreferenceValues = serde_json::from_value(json!({"skin": "vibrancy", "showThinking": true})).unwrap();
        assert_eq!(parsed.skin, Some(ThemeSkin::Vibrancy));
        assert!(serde_json::from_value::<AppPreferenceValues>(json!({"zoom": 2})).is_err());
    }

    #[test]
    fn a_retired_effort_selector_is_read_and_dropped_not_rejected() {
        // Records and clients from before the single effort control still
        // carry the key. Rejecting it would reset theme and skin with it.
        let record: AppPreferences = serde_json::from_value(json!({"revision": 3, "values": {"theme": "dark", "effortSelector": "list"}})).unwrap();
        assert_eq!(record.values.theme, Some(ThemePreference::Dark));
        assert_eq!(serde_json::to_value(&record.values).unwrap(), json!({"theme": "dark"}));
        let params: SaveAppPreferencesParams = serde_json::from_value(json!({"values": {"effortSelector": "sentence"}})).unwrap();
        assert_eq!(params.values, AppPreferenceValues::default());
    }
}
