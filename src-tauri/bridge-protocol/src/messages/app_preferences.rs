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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EffortSelectorStyle {
    Slider,
    Sentence,
    List,
}

/// Absent means never set: the app keeps its built-in default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AppPreferenceValues {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemePreference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<ThemeSkin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_selector: Option<EffortSelectorStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_expand_edits: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_are_lowercase_and_unset_keys_stay_off_the_wire() {
        let values = AppPreferenceValues { theme: Some(ThemePreference::Light), show_thinking: Some(false), ..Default::default() };
        assert_eq!(serde_json::to_value(&values).unwrap(), json!({"theme": "light", "showThinking": false}));
        let parsed: AppPreferenceValues = serde_json::from_value(json!({"skin": "vibrancy", "effortSelector": "list"})).unwrap();
        assert_eq!(parsed.skin, Some(ThemeSkin::Vibrancy));
        assert_eq!(parsed.effort_selector, Some(EffortSelectorStyle::List));
        assert!(serde_json::from_value::<AppPreferenceValues>(json!({"zoom": 2})).is_err());
    }
}
