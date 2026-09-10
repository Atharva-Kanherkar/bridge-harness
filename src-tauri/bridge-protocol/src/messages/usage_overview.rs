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
pub struct UsageOverviewSnapshot {
    pub schema_version: u32,
    pub generated_at: i64,
    pub provider: String,
    /// Public account identity returned by the provider; never a credential.
    pub account: Option<String>,
    pub plan: Option<String>,
    pub observed_at: Option<i64>,
    pub windows: Vec<UsageQuotaWindow>,
    pub today: UsagePeriodOverview,
    pub month: UsagePeriodOverview,
    /// The ledger currently covers this device, not an entire billing account.
    pub coverage: String,
    pub error: Option<String>,
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
    Session,
    Weekly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuBarSettings {
    pub schema_version: u32,
    pub enabled: bool,
    pub codex_enabled: bool,
    pub display_mode: MenuBarDisplayMode,
    pub quota_window: MenuBarQuotaWindow,
    pub show_account: bool,
    pub show_tokens: bool,
    pub show_cost: bool,
    /// Zero means manual; otherwise 60, 300, 900, or 1800 seconds.
    pub refresh_seconds: u64,
}

impl Default for MenuBarSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            codex_enabled: true,
            display_mode: MenuBarDisplayMode::Remaining,
            quota_window: MenuBarQuotaWindow::Session,
            show_account: true,
            show_tokens: true,
            show_cost: true,
            refresh_seconds: 300,
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

    /// This exact fixture is also decoded by the Swift presentation tests.
    #[test]
    fn native_fixture_matches_the_versioned_wire_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/menu-bar-presentation.json"
        ))
        .unwrap();
        let settings: MenuBarSettings =
            serde_json::from_value(fixture["settings"].clone()).unwrap();
        let usage: UsageOverviewSnapshot =
            serde_json::from_value(fixture["usage"].clone()).unwrap();
        assert_eq!(serde_json::to_value(settings).unwrap(), fixture["settings"]);
        assert_eq!(serde_json::to_value(&usage).unwrap(), fixture["usage"]);
        assert_eq!(usage.windows[0].used_percent.value, Some(0.0));
        assert_eq!(
            usage.windows[1].used_percent.status,
            UsageMetricStatus::Stale
        );
        assert_eq!(usage.month.cost_microusd.value, None);
    }
}
