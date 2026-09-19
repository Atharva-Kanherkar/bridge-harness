//! Versioned provider usage shared by desktop and menu presentation. A missing
//! value is never zero; provenance and freshness are independent dimensions.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageMetricSource {
    Reported,
    Measured,
    Estimated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageMetricStatus {
    Current,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetric {
    pub value: Option<f64>,
    pub source: Option<UsageMetricSource>,
    pub status: UsageMetricStatus,
}

impl UsageMetric {
    pub fn unavailable() -> Self {
        Self {
            value: None,
            source: None,
            status: UsageMetricStatus::Unavailable,
        }
    }
    pub fn known(value: f64, source: UsageMetricSource) -> Self {
        if !value.is_finite() || value < 0.0 {
            return Self::unavailable();
        }
        Self {
            value: Some(value),
            source: Some(source),
            status: UsageMetricStatus::Current,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuotaWindow {
    pub id: String,
    pub label: String,
    pub used_percent: UsageMetric,
    pub resets_at: Option<i64>,
    pub window_minutes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageModelOverview {
    pub model: String,
    pub input_tokens: UsageMetric,
    pub output_tokens: UsageMetric,
    pub cache_tokens: UsageMetric,
    pub total_tokens: UsageMetric,
    pub cost_microusd: UsageMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsagePeriodOverview {
    pub tokens: UsageMetric,
    pub cost_microusd: UsageMetric,
    pub models: Vec<UsageModelOverview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageDailyOverview {
    /// Local calendar day in the same time zone as today/month aggregation.
    pub day: String,
    pub usage: UsagePeriodOverview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageOverviewSnapshot {
    pub schema_version: u32,
    pub generated_at: i64,
    pub provider: String,
    /// Public account identity returned by the provider; never a credential.
    pub account: Option<String>,
    pub plan: Option<String>,
    pub observed_at: Option<i64>,
    /// Public collector name, not an account identity or authentication token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_source: Option<String>,
    pub windows: Vec<UsageQuotaWindow>,
    /// Provider-reported account amounts, separate from the device ledger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_metrics: Vec<UsageAccountMetric>,
    pub today: UsagePeriodOverview,
    pub month: UsagePeriodOverview,
    /// Up to 30 local days with recorded usage. Missing days are not measured zero.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub daily: Vec<UsageDailyOverview>,
    /// The ledger currently covers this device, not an entire billing account.
    pub coverage: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageAccountMetric {
    pub id: String,
    pub label: String,
    /// Amounts use micro-USD, like the shared ledger.
    pub value: UsageMetric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageOverviews {
    pub schema_version: u32,
    pub generated_at: i64,
    pub providers: Vec<UsageOverviewSnapshot>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarProvider {
    #[default]
    Codex,
    Claude,
    Cursor,
    OpenCode,
}

impl MenuBarProvider {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Cursor, Self::OpenCode];
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarDisplayMode {
    Icon,
    Used,
    Remaining,
    Cost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarQuotaWindow {
    Auto,
    Session,
    Weekly,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarQuotaDisplayMode {
    #[default]
    Used,
    Remaining,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarIconStyle {
    #[default]
    Bridge,
    Meter,
}

/// Presentation tokens only. No expressions, credentials, scripts or network access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MenuBarLayoutToken {
    Icon,
    Provider,
    Used,
    Remaining,
    WeeklyUsed,
    WeeklyRemaining,
    FiveHourUsed,
    FiveHourRemaining,
    Reset,
    TodayCost,
    Dot,
    Space,
}

fn default_true() -> bool {
    true
}

fn default_pinned_providers() -> Vec<MenuBarProvider> {
    vec![
        MenuBarProvider::Codex,
        MenuBarProvider::Claude,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuBarSettings {
    pub schema_version: u32,
    pub enabled: bool,
    pub codex_enabled: bool,
    #[serde(default)]
    pub claude_enabled: bool,
    #[serde(default)]
    pub cursor_enabled: bool,
    #[serde(default)]
    pub opencode_enabled: bool,
    #[serde(default)]
    pub selected_provider: MenuBarProvider,
    /// Unique favorites in tab order. Pinning never enables account collection.
    #[serde(default = "default_pinned_providers")]
    pub pinned_providers: Vec<MenuBarProvider>,
    #[serde(default)]
    pub opencode_workspace: Option<String>,
    pub display_mode: MenuBarDisplayMode,
    /// Quota bar fill is independent of the status item's text layout.
    #[serde(default)]
    pub quota_display_mode: MenuBarQuotaDisplayMode,
    #[serde(default = "default_true")]
    pub open_to_overview: bool,
    #[serde(default)]
    pub icon_style: MenuBarIconStyle,
    /// Empty uses display_mode. Custom layouts contain at most two lines.
    #[serde(default)]
    pub status_layout: Vec<Vec<MenuBarLayoutToken>>,
    pub quota_window: MenuBarQuotaWindow,
    pub show_account: bool,
    pub show_tokens: bool,
    pub show_cost: bool,
    #[serde(default = "default_true")]
    pub show_history: bool,
    #[serde(default = "default_true")]
    pub show_overview_summary: bool,
    #[serde(default)]
    pub separate_provider_icons: bool,
    /// Zero means manual; otherwise 60, 300, 900, or 1800 seconds.
    pub refresh_seconds: u64,
}

impl Default for MenuBarSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            codex_enabled: true,
            claude_enabled: false,
            cursor_enabled: false,
            opencode_enabled: false,
            selected_provider: MenuBarProvider::Codex,
            pinned_providers: default_pinned_providers(),
            opencode_workspace: None,
            display_mode: MenuBarDisplayMode::Used,
            quota_display_mode: MenuBarQuotaDisplayMode::Used,
            open_to_overview: true,
            icon_style: MenuBarIconStyle::Bridge,
            status_layout: vec![],
            quota_window: MenuBarQuotaWindow::Auto,
            show_account: true,
            show_tokens: true,
            show_cost: true,
            show_history: true,
            show_overview_summary: true,
            separate_provider_icons: false,
            refresh_seconds: 300,
        }
    }
}

impl MenuBarSettings {
    pub fn provider_visible(&self, provider: MenuBarProvider) -> bool {
        self.pinned_providers.contains(&provider) || self.provider_enabled(provider)
    }

    pub fn provider_enabled(&self, provider: MenuBarProvider) -> bool {
        match provider {
            MenuBarProvider::Codex => self.codex_enabled,
            MenuBarProvider::Claude => self.claude_enabled,
            MenuBarProvider::Cursor => self.cursor_enabled,
            MenuBarProvider::OpenCode => self.opencode_enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMenuBarSettingsParams {
    pub settings: MenuBarSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_codex_preferences_migrate_without_enabling_new_collectors() {
        let original = serde_json::json!({"schemaVersion":1,"enabled":false,"codexEnabled":true,"displayMode":"used","quotaWindow":"weekly","showAccount":false,"showTokens":false,"showCost":true,"refreshSeconds":900});
        let settings: MenuBarSettings = serde_json::from_value(original).unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.display_mode, MenuBarDisplayMode::Used);
        assert_eq!(settings.refresh_seconds, 900);
        assert_eq!(settings.selected_provider, MenuBarProvider::Codex);
        assert_eq!(
            settings.pinned_providers,
            vec![
                MenuBarProvider::Codex,
                MenuBarProvider::Claude
            ]
        );
        for provider in MenuBarProvider::ALL.into_iter().skip(1) {
            assert!(!settings.provider_enabled(provider));
        }
        assert!(!settings.provider_visible(MenuBarProvider::Cursor));
        assert!(!settings.provider_visible(MenuBarProvider::OpenCode));
        assert!(settings.opencode_workspace.is_none());
        assert_eq!(settings.quota_display_mode, MenuBarQuotaDisplayMode::Used);
        assert!(settings.open_to_overview && settings.show_history);
        assert!(settings.show_overview_summary);
        assert!(!settings.separate_provider_icons);
        assert!(settings.status_layout.is_empty());
    }

    #[test]
    fn connection_debug_output_redacts_the_session_cookie() {
        let session = SaveOpencodeUsageSessionParams {
            cookie: "auth=secret-session".into(),
            workspace: "wrk_fixture".into(),
        };
        let debug = format!("{session:?}");
        assert!(!debug.contains("secret-session"));
        assert!(debug.contains("[REDACTED]"));
    }

    /// This exact fixture is also decoded by the Swift presentation tests.
    #[test]
    fn native_fixture_matches_the_versioned_wire_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/menu-bar-presentation.json"
        ))
        .unwrap();
        let settings: MenuBarSettings =
            serde_json::from_value(fixture["settings"].clone()).unwrap();
        let group: ProviderUsageOverviews =
            serde_json::from_value(fixture["usage"].clone()).unwrap();
        assert_eq!(serde_json::to_value(settings).unwrap(), fixture["settings"]);
        assert_eq!(serde_json::to_value(&group).unwrap(), fixture["usage"]);
        let usage = &group.providers[0];
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(
            usage.windows[1].used_percent.status,
            UsageMetricStatus::Stale
        );
        assert_eq!(usage.month.cost_microusd.value, None);
    }
}

/// Shell-to-backend only. This session is written to Keychain and never returned
/// by a settings, usage, or credential-read API.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveOpencodeUsageSessionParams {
    pub cookie: String,
    pub workspace: String,
}
impl std::fmt::Debug for SaveOpencodeUsageSessionParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveOpencodeUsageSessionParams")
            .field("cookie", &"[REDACTED]")
            .field("workspace", &self.workspace)
            .finish()
    }
}
