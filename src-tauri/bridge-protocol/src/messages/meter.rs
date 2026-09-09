//! Menu-bar meter: provider registry plus the pace detail CodexBar shows per
//! quota window. Mirrors `bridge_core::meter`.
//!
//! Meter math is ported from steipete/CodexBar (MIT): `UsagePace.weekly`
//! (`Sources/CodexBarCore/UsagePace.swift`) and the adaptive table
//! (`Sources/CodexBar/AdaptiveRefreshPolicy.swift`, `docs/refresh-loop.md`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One provider in the meter registry. Mirrors `bridge_core::meter::MeterProviderEntry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeterProviderEntry {
    pub id: String,
    pub label: String,
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_source: Option<String>,
}

/// Static registry payload for `meter/get_meter_snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeterRegistry {
    pub providers: Vec<MeterProviderEntry>,
    pub adaptive_default_seconds: i64,
    pub nominal_interval_seconds: i64,
    pub attribution: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;

    #[test]
    fn meter_registry_round_trips() {
        let registry = MeterRegistry {
            providers: vec![MeterProviderEntry {
                id: "codex".into(),
                label: "Codex".into(),
                supported: true,
                planned_source: None,
            }],
            adaptive_default_seconds: 300,
            nominal_interval_seconds: 300,
            attribution: "test".into(),
        };
        assert_eq!(round_trip(&registry), registry);
    }
}
