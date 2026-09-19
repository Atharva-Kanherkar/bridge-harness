//! Read-only account collectors. Authentication and transport stay out of both UIs.
mod claude;
mod claude_cli;
mod claude_sdk;
pub mod credentials;
mod cursor;
mod http;
mod opencode;
mod opencode_go;

pub fn shutdown() {
    claude_cli::shutdown();
}

use bridge_protocol::messages::{
    MenuBarProvider, MenuBarSettings, UsageAccountMetric, UsageDailyOverview, UsageMetric,
    UsageMetricSource, UsagePeriodOverview, UsageQuotaWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A failed read may authorize stale Cursor data only after the current local
/// session has identified the same account. Credentials never enter the cache.
#[derive(Debug)]
pub(crate) struct AccountReadError {
    pub message: String,
    pub retry_account_scope: Option<String>,
}

impl From<String> for AccountReadError {
    fn from(message: String) -> Self {
        Self { message, retry_account_scope: None }
    }
}

impl From<&str> for AccountReadError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl AccountReadError {
    fn for_account(error: http::RequestError, scope: &str) -> Self {
        Self {
            message: error.message,
            retry_account_scope: error.retryable.then(|| scope.to_owned()),
        }
    }
}

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
    /// Exact daily dashboard buckets, never synthetic transcript records.
    /// Older caches stay usable by the menu, but need a refresh for the main report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<AccountHistoryBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountHistoryBreakdown {
    pub time_zone: String,
    pub since_day: String,
    pub buckets: Vec<crate::usage_summary::UsageBucket>,
}

pub(crate) fn read_interactive(
    core: &crate::BridgeCore,
    provider: MenuBarProvider,
    settings: &MenuBarSettings,
) -> Result<AccountUsage, AccountReadError> {
    match provider {
        MenuBarProvider::Claude => claude::read_interactive(core).map_err(Into::into),
        _ => read(core, provider, settings),
    }
}

pub(crate) fn read(
    core: &crate::BridgeCore,
    provider: MenuBarProvider,
    settings: &MenuBarSettings,
) -> Result<AccountUsage, AccountReadError> {
    match provider {
        MenuBarProvider::Claude => claude::read(core).map_err(Into::into),
        MenuBarProvider::Cursor => cursor::read(),
        MenuBarProvider::OpenCode => opencode::read(settings.opencode_workspace.as_deref()).map_err(Into::into),
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
