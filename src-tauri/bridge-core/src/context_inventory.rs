//! Fail-closed observations of context owned by provider adapters.
//!
//! These values describe bytes and catalogs assembled outside
//! [`crate::prompt_compiler::PromptCompiler`]. In particular, compiler tool
//! schemas are Bridge-authored prompt bytes and are never evidence of the tools
//! a provider actually presented to a model.

use crate::{secret_interception, BridgeError};
use serde::Serialize;
use std::{collections::HashSet, sync::Mutex};

pub const MAX_CONTEXT_ITEMS: usize = 128;
pub const MAX_CONTEXT_NAME_BYTES: usize = 160;
pub const MAX_CONTEXT_SIZE_VALUE: u64 = 1_000_000_000;
pub const MAX_RUNTIME_CONTEXT_INVENTORIES: usize = 130;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextSegmentClass {
    ProviderBaseInstructions,
    ToolSchemas,
    McpDynamicTools,
    SkillsPlugins,
    AgentDefinitions,
}

impl ContextSegmentClass {
    pub const ALL: [Self; 5] = [
        Self::ProviderBaseInstructions,
        Self::ToolSchemas,
        Self::McpDynamicTools,
        Self::SkillsPlugins,
        Self::AgentDefinitions,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextInventoryScope {
    Catalog,
    TurnPresented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextLifecyclePhase {
    Start,
    Resume,
    PerTurn,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextObservedSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    pub capped: bool,
}

impl ContextObservedSize {
    pub fn bounded(item_count: Option<u64>, bytes: Option<u64>, tokens: Option<u64>) -> Self {
        let capped = item_count.is_some_and(|value| value > MAX_CONTEXT_ITEMS as u64)
            || bytes.is_some_and(|value| value > MAX_CONTEXT_SIZE_VALUE)
            || tokens.is_some_and(|value| value > MAX_CONTEXT_SIZE_VALUE);
        Self {
            item_count: item_count.map(|value| value.min(MAX_CONTEXT_ITEMS as u64)),
            bytes: bytes.map(|value| value.min(MAX_CONTEXT_SIZE_VALUE)),
            tokens: tokens.map(|value| value.min(MAX_CONTEXT_SIZE_VALUE)),
            capped,
        }
    }

    fn rebound(&mut self) {
        let bounded = Self::bounded(self.item_count, self.bytes, self.tokens);
        self.item_count = bounded.item_count;
        self.bytes = bounded.bytes;
        self.tokens = bounded.tokens;
        self.capped |= bounded.capped;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ContextObservationProvenance {
    Reported {
        size: ContextObservedSize,
    },
    Measured {
        size: ContextObservedSize,
    },
    Estimated {
        size: ContextObservedSize,
        method: String,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSegmentObservation {
    pub segment_class: ContextSegmentClass,
    pub names: Vec<String>,
    pub names_truncated: bool,
    pub provenance: ContextObservationProvenance,
}

impl ContextSegmentObservation {
    pub fn reported(
        segment_class: ContextSegmentClass,
        names: impl IntoIterator<Item = impl AsRef<str>>,
        size: ContextObservedSize,
    ) -> Self {
        Self::observed(
            segment_class,
            names,
            ContextObservationProvenance::Reported { size },
        )
    }

    pub fn measured(
        segment_class: ContextSegmentClass,
        names: impl IntoIterator<Item = impl AsRef<str>>,
        size: ContextObservedSize,
    ) -> Self {
        Self::observed(
            segment_class,
            names,
            ContextObservationProvenance::Measured { size },
        )
    }

    pub fn estimated(
        segment_class: ContextSegmentClass,
        names: impl IntoIterator<Item = impl AsRef<str>>,
        size: ContextObservedSize,
        method: impl Into<String>,
    ) -> Self {
        Self::observed(
            segment_class,
            names,
            ContextObservationProvenance::Estimated {
                size,
                method: bounded_text(&method.into()).0,
            },
        )
    }

    pub fn unavailable(segment_class: ContextSegmentClass, reason: impl Into<String>) -> Self {
        Self::unavailable_with_names(segment_class, std::iter::empty::<&str>(), reason)
    }

    pub fn unavailable_with_names(
        segment_class: ContextSegmentClass,
        names: impl IntoIterator<Item = impl AsRef<str>>,
        reason: impl Into<String>,
    ) -> Self {
        let (reason, _) = bounded_text(&reason.into());
        Self::observed(
            segment_class,
            names,
            ContextObservationProvenance::Unavailable { reason },
        )
    }

    fn observed(
        segment_class: ContextSegmentClass,
        names: impl IntoIterator<Item = impl AsRef<str>>,
        provenance: ContextObservationProvenance,
    ) -> Self {
        let (bounded, names_truncated) = Self::observed_names(names);
        let mut observation = Self {
            segment_class,
            names: bounded,
            names_truncated,
            provenance,
        };
        observation.normalize_for_storage();
        observation
    }

    fn normalize_for_storage(&mut self) {
        let original_names = std::mem::take(&mut self.names);
        let rebuilt = Self::observed_names(original_names);
        self.names = rebuilt.0;
        self.names_truncated |= rebuilt.1;
        match &mut self.provenance {
            ContextObservationProvenance::Reported { size }
            | ContextObservationProvenance::Measured { size } => size.rebound(),
            ContextObservationProvenance::Estimated { size, method } => {
                size.rebound();
                *method = bounded_text(method).0;
            }
            ContextObservationProvenance::Unavailable { reason } => {
                *reason = bounded_text(reason).0;
            }
        }
    }

    fn observed_names(names: impl IntoIterator<Item = impl AsRef<str>>) -> (Vec<String>, bool) {
        let mut names_truncated = false;
        let mut bounded = Vec::new();
        for name in names.into_iter().take(MAX_CONTEXT_ITEMS + 1) {
            if bounded.len() == MAX_CONTEXT_ITEMS {
                names_truncated = true;
                break;
            }
            let (name, truncated) = bounded_text(name.as_ref());
            names_truncated |= truncated;
            bounded.push(name);
        }
        (bounded, names_truncated)
    }

    fn validate(&self) -> Result<(), BridgeError> {
        match &self.provenance {
            ContextObservationProvenance::Unavailable { reason } if reason.trim().is_empty() => {
                Err(BridgeError::Invalid(format!(
                    "Unavailable {:?} context requires a reason",
                    self.segment_class
                )))
            }
            ContextObservationProvenance::Estimated { method, .. } if method.trim().is_empty() => {
                Err(BridgeError::Invalid(format!(
                    "Estimated {:?} context requires a method",
                    self.segment_class
                )))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterContextInventory {
    pub adapter_id: String,
    pub scope: ContextInventoryScope,
    pub lifecycle_phase: ContextLifecyclePhase,
    pub observations: Vec<ContextSegmentObservation>,
}

impl AdapterContextInventory {
    pub fn new(
        adapter_id: impl Into<String>,
        scope: ContextInventoryScope,
        lifecycle_phase: ContextLifecyclePhase,
        mut observations: Vec<ContextSegmentObservation>,
    ) -> Result<Self, BridgeError> {
        let (adapter_id, _) = bounded_text(&adapter_id.into());
        if adapter_id.trim().is_empty() {
            return Err(BridgeError::Invalid(
                "Context inventory adapter id cannot be empty".into(),
            ));
        }
        let mut classes = HashSet::new();
        for observation in &mut observations {
            observation.normalize_for_storage();
            observation.validate()?;
            if !classes.insert(observation.segment_class) {
                return Err(BridgeError::Invalid(format!(
                    "Context inventory for {adapter_id} duplicates {:?}",
                    observation.segment_class
                )));
            }
        }
        let missing = ContextSegmentClass::ALL
            .into_iter()
            .filter(|class| !classes.contains(class))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(BridgeError::Invalid(format!(
                "Context inventory for {adapter_id} is missing {missing:?}"
            )));
        }
        Ok(Self {
            adapter_id,
            scope,
            lifecycle_phase,
            observations,
        })
    }
}

pub const CONTEXT_INVENTORY_ADAPTERS: [&str; 3] = ["claude", "codex", "opencode"];

/// Static fail-closed contract used by registry conformance and by callers that
/// need the lifecycle shape before a provider process exists.
pub fn adapter_context_inventory_contract(
    adapter_id: &str,
    lifecycle_phase: ContextLifecyclePhase,
) -> Result<Vec<AdapterContextInventory>, BridgeError> {
    match adapter_id {
        "claude" => crate::claude_adapter::claude_context_inventory(
            lifecycle_phase,
            &crate::marketplace::ClaudeSdkConfiguration::default(),
        ),
        "codex" => crate::codex_adapter::codex_context_inventory(lifecycle_phase),
        "opencode" => crate::opencode_adapter::opencode_context_inventory(lifecycle_phase),
        unknown => Err(BridgeError::Invalid(format!(
            "Adapter {unknown} has no provider context inventory contract"
        ))),
    }
}

pub fn record_runtime_inventory(
    target: &Mutex<Vec<AdapterContextInventory>>,
    inventories: impl IntoIterator<Item = AdapterContextInventory>,
) {
    let mut target = target.lock().unwrap();
    for inventory in inventories {
        if target.len() >= MAX_RUNTIME_CONTEXT_INVENTORIES {
            let removable = target
                .iter()
                .position(|existing| existing.lifecycle_phase == ContextLifecyclePhase::PerTurn)
                .unwrap_or(0);
            target.remove(removable);
        }
        target.push(inventory);
    }
}

fn bounded_text(value: &str) -> (String, bool) {
    let lower = value.to_ascii_lowercase();
    let sanitized = if lower.contains("/credential-proxy/") || lower.contains("x-bridge-proxy-auth")
    {
        "[redacted]".to_owned()
    } else {
        secret_interception::sanitize(value).text
    };
    if sanitized.len() <= MAX_CONTEXT_NAME_BYTES {
        return (sanitized, false);
    }
    let mut end = MAX_CONTEXT_NAME_BYTES;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = sanitized[..end].to_owned();
    bounded.push('…');
    (bounded, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unavailable(class: ContextSegmentClass) -> ContextSegmentObservation {
        ContextSegmentObservation::unavailable(class, "provider does not expose this segment")
    }

    fn complete(observation: ContextSegmentObservation) -> Vec<ContextSegmentObservation> {
        ContextSegmentClass::ALL
            .into_iter()
            .map(|class| {
                if class == observation.segment_class {
                    observation.clone()
                } else {
                    unavailable(class)
                }
            })
            .collect()
    }

    #[test]
    fn provenance_states_serialize_with_camel_case_fields() {
        let cases = [
            ContextSegmentObservation::reported(
                ContextSegmentClass::ToolSchemas,
                ["reported"],
                ContextObservedSize::bounded(Some(1), None, Some(2)),
            ),
            ContextSegmentObservation::measured(
                ContextSegmentClass::ToolSchemas,
                ["measured"],
                ContextObservedSize::bounded(Some(1), Some(8), None),
            ),
            ContextSegmentObservation::estimated(
                ContextSegmentClass::ToolSchemas,
                ["estimated"],
                ContextObservedSize::bounded(None, None, Some(2)),
                "measured bytes divided by four",
            ),
            unavailable(ContextSegmentClass::ToolSchemas),
        ];
        let states = cases
            .iter()
            .map(|case| serde_json::to_value(case).unwrap()["provenance"]["state"].clone())
            .collect::<Vec<_>>();
        assert_eq!(states, ["reported", "measured", "estimated", "unavailable"]);
        assert_eq!(
            serde_json::to_value(&cases[0]).unwrap()["segmentClass"],
            "toolSchemas"
        );
    }

    #[test]
    fn unavailable_requires_a_reason() {
        let observations = complete(ContextSegmentObservation::unavailable(
            ContextSegmentClass::ToolSchemas,
            "   ",
        ));
        assert!(AdapterContextInventory::new(
            "codex",
            ContextInventoryScope::TurnPresented,
            ContextLifecyclePhase::Start,
            observations,
        )
        .is_err());
    }

    #[test]
    fn names_counts_and_secrets_are_bounded_before_storage() {
        let secret = "sk-proj-abcdefghijklmnopqrstuvwxyz123456";
        let names = (0..MAX_CONTEXT_ITEMS + 2)
            .map(|index| format!("plugin-{index}-{secret}-{}", "界".repeat(100)))
            .collect::<Vec<_>>();
        let observation = ContextSegmentObservation::reported(
            ContextSegmentClass::SkillsPlugins,
            &names,
            ContextObservedSize::bounded(
                Some((MAX_CONTEXT_ITEMS + 2) as u64),
                Some(MAX_CONTEXT_SIZE_VALUE + 1),
                None,
            ),
        );
        assert_eq!(observation.names.len(), MAX_CONTEXT_ITEMS);
        assert!(observation.names_truncated);
        assert!(observation.names.iter().all(|name| !name.contains(secret)));
        assert!(observation
            .names
            .iter()
            .all(|name| name.is_char_boundary(name.len())));
        let ContextObservationProvenance::Reported { size } = observation.provenance else {
            panic!("expected reported provenance")
        };
        assert_eq!(size.item_count, Some(MAX_CONTEXT_ITEMS as u64));
        assert_eq!(size.bytes, Some(MAX_CONTEXT_SIZE_VALUE));
        assert!(size.capped);

        let direct = ContextSegmentObservation {
            segment_class: ContextSegmentClass::SkillsPlugins,
            names,
            names_truncated: false,
            provenance: ContextObservationProvenance::Reported {
                size: ContextObservedSize {
                    item_count: Some(u64::MAX),
                    bytes: Some(u64::MAX),
                    tokens: Some(u64::MAX),
                    capped: false,
                },
            },
        };
        let inventory = AdapterContextInventory::new(
            "claude",
            ContextInventoryScope::Catalog,
            ContextLifecyclePhase::Start,
            complete(direct),
        )
        .unwrap();
        let stored = inventory
            .observations
            .iter()
            .find(|observation| observation.segment_class == ContextSegmentClass::SkillsPlugins)
            .unwrap();
        assert_eq!(stored.names.len(), MAX_CONTEXT_ITEMS);
        assert!(stored.names.iter().all(|name| !name.contains(secret)));
        let ContextObservationProvenance::Reported { size } = &stored.provenance else {
            panic!("expected reported provenance")
        };
        assert_eq!(size.bytes, Some(MAX_CONTEXT_SIZE_VALUE));
        assert!(size.capped);
    }

    #[test]
    fn every_inventory_requires_exactly_one_observation_per_segment_class() {
        let mut missing = ContextSegmentClass::ALL
            .into_iter()
            .take(4)
            .map(unavailable)
            .collect::<Vec<_>>();
        assert!(AdapterContextInventory::new(
            "claude",
            ContextInventoryScope::Catalog,
            ContextLifecyclePhase::Start,
            missing.clone(),
        )
        .is_err());
        missing.push(unavailable(ContextSegmentClass::ProviderBaseInstructions));
        assert!(AdapterContextInventory::new(
            "claude",
            ContextInventoryScope::Catalog,
            ContextLifecyclePhase::Start,
            missing,
        )
        .is_err());
    }

    #[test]
    fn estimated_values_require_an_explicit_method() {
        let observations = complete(ContextSegmentObservation::estimated(
            ContextSegmentClass::ProviderBaseInstructions,
            ["provider base"],
            ContextObservedSize::bounded(None, Some(12), Some(3)),
            "",
        ));
        assert!(AdapterContextInventory::new(
            "opencode",
            ContextInventoryScope::TurnPresented,
            ContextLifecyclePhase::PerTurn,
            observations,
        )
        .is_err());
    }

    #[test]
    fn provider_totals_are_not_reverse_allocated() {
        let provider_total_tokens = 10_000_u64;
        let inventory = AdapterContextInventory::new(
            "codex",
            ContextInventoryScope::TurnPresented,
            ContextLifecyclePhase::PerTurn,
            ContextSegmentClass::ALL
                .into_iter()
                .map(unavailable)
                .collect(),
        )
        .unwrap();
        assert!(inventory.observations.iter().all(|observation| matches!(
            observation.provenance,
            ContextObservationProvenance::Unavailable { .. }
        )));
        assert_eq!(provider_total_tokens, 10_000);
    }

    #[test]
    fn provider_tool_inventory_does_not_read_prompt_compiler_tool_schemas() {
        let compiled = crate::prompt_compiler::PromptCompiler::new("direct")
            .compile()
            .unwrap();
        assert!(compiled.stable_prefix.contains("\"toolSchemas\":{}"));
        for adapter_id in CONTEXT_INVENTORY_ADAPTERS {
            let inventories =
                adapter_context_inventory_contract(adapter_id, ContextLifecyclePhase::PerTurn)
                    .unwrap();
            let tool_observation = inventories[0]
                .observations
                .iter()
                .find(|observation| observation.segment_class == ContextSegmentClass::ToolSchemas)
                .unwrap();
            assert!(matches!(
                tool_observation.provenance,
                ContextObservationProvenance::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn every_registered_adapter_has_a_fail_closed_context_inventory_contract() {
        let registry = crate::adapters::AdapterRegistry::built_in().unwrap();
        let mut registered = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        registered.sort();
        let mut contracted = CONTEXT_INVENTORY_ADAPTERS.map(str::to_owned).to_vec();
        contracted.sort();
        assert_eq!(registered, contracted);

        for adapter_id in registered {
            for phase in [
                ContextLifecyclePhase::Start,
                ContextLifecyclePhase::Resume,
                ContextLifecyclePhase::PerTurn,
            ] {
                let inventories = adapter_context_inventory_contract(&adapter_id, phase).unwrap();
                assert!(!inventories.is_empty(), "{adapter_id} {phase:?}");
                assert!(inventories.iter().all(|inventory| {
                    inventory.adapter_id == adapter_id
                        && inventory.lifecycle_phase == phase
                        && inventory.observations.len() == ContextSegmentClass::ALL.len()
                }));
                assert!(inventories
                    .iter()
                    .any(|inventory| inventory.scope == ContextInventoryScope::TurnPresented));
                if phase != ContextLifecyclePhase::PerTurn {
                    assert!(inventories
                        .iter()
                        .any(|inventory| inventory.scope == ContextInventoryScope::Catalog));
                }
            }
        }
        assert!(adapter_context_inventory_contract(
            "new-adapter-without-contract",
            ContextLifecyclePhase::Start,
        )
        .is_err());
    }

    #[test]
    fn per_turn_runtime_inventory_is_bounded_without_dropping_startup_catalog() {
        let startup =
            adapter_context_inventory_contract("codex", ContextLifecyclePhase::Start).unwrap();
        let target = Mutex::new(startup);
        for _ in 0..MAX_RUNTIME_CONTEXT_INVENTORIES + 10 {
            record_runtime_inventory(
                &target,
                adapter_context_inventory_contract("codex", ContextLifecyclePhase::PerTurn)
                    .unwrap(),
            );
        }
        let inventories = target.lock().unwrap();
        assert_eq!(inventories.len(), MAX_RUNTIME_CONTEXT_INVENTORIES);
        assert!(inventories.iter().any(|inventory| {
            inventory.scope == ContextInventoryScope::Catalog
                && inventory.lifecycle_phase == ContextLifecyclePhase::Start
        }));
    }
}
