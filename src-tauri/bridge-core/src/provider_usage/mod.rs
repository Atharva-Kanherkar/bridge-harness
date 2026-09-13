//! Read-only account collectors. Authentication and transport stay out of both UIs.
mod claude;
mod claude_cli;
pub mod credentials;
mod cursor;
mod http;
mod opencode;

pub fn shutdown() {
    claude_cli::shutdown();
}

use bridge_protocol::messages::{
    MenuBarProvider, MenuBarSettings, UsageAccountMetric, UsageDailyOverview, UsageMetric,
    UsageMetricSource, UsagePeriodOverview, UsageQuotaWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct AccountUsage {
    pub account: Option<String>,
    pub plan: Option<String>,
    pub observed_at: i64,
    pub windows: Vec<UsageQuotaWindow>,
    pub metrics: Vec<UsageAccountMetric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Internal fingerprint of a freshly verified account; never an auth token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_scope: Option<String>,
    /// Remote account history stays separate from the device-local usage ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<AccountHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountHistory {
    pub account_scope: String,
    pub observed_at: i64,
    #[serde(default)]
    pub through_day: String,
    pub today: UsagePeriodOverview,
    pub month: UsagePeriodOverview,
    pub daily: Vec<UsageDailyOverview>,
    pub coverage: String,
}

pub(crate) fn read_interactive(
    core: &crate::BridgeCore,
    provider: MenuBarProvider,
    settings: &MenuBarSettings,
) -> Result<AccountUsage, String> {
    match provider {
        MenuBarProvider::Claude => claude::read_interactive(core),
        _ => read(provider, settings),
    }
}

pub(crate) fn read(
    provider: MenuBarProvider,
    settings: &MenuBarSettings,
) -> Result<AccountUsage, String> {
    match provider {
        MenuBarProvider::Claude => claude::read(),
        MenuBarProvider::Cursor => cursor::read(),
        MenuBarProvider::OpenCode => opencode::read(settings.opencode_workspace.as_deref()),
        MenuBarProvider::Codex => Err("Codex uses its app-server collector".into()),
    }
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite() && *v >= 0.0)
}
fn timestamp(value: &Value) -> Option<i64> {
    value.as_i64().filter(|v| *v > 0).or_else(|| {
        value
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|v| v.timestamp())
    })
}
fn metric(value: Option<f64>) -> UsageMetric {
    value
        .map(|v| UsageMetric::known(v, UsageMetricSource::Reported))
        .unwrap_or_else(UsageMetric::unavailable)
}
fn amount(id: &str, label: &str, value_microusd: Option<f64>) -> UsageAccountMetric {
    UsageAccountMetric {
        id: id.into(),
        label: label.into(),
        value: metric(value_microusd),
    }
}
fn window(
    id: &str,
    label: &str,
    percent: Option<f64>,
    resets_at: Option<i64>,
    minutes: Option<i64>,
) -> UsageQuotaWindow {
    UsageQuotaWindow {
        id: id.into(),
        label: label.into(),
        used_percent: metric(percent),
        resets_at,
        window_minutes: minutes,
    }
}
fn public_text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().filter(|c| !c.is_control()).take(160).collect())
}
