//! Portable, versioned continuation contract for cross-harness work.

use serde::{Deserialize, Serialize};

pub const HANDOFF_PACKET_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandoffPacket {
    pub schema_version: u32,
    pub trace_id: String,
    pub source_harness: String,
    pub target_harness: String,
    pub repository_revision: Option<String>,
    pub permission_mode: String,
    pub budget: serde_json::Value,
    pub evidence_ids: Vec<String>,
    pub parent_entry_id: Option<String>,
    pub output_contract: String,
}

impl HandoffPacket {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != HANDOFF_PACKET_SCHEMA_VERSION {
            return Err(format!(
                "unsupported handoff packet schema {}",
                self.schema_version
            ));
        }
        for (field, value) in [
            ("traceId", self.trace_id.as_str()),
            ("sourceHarness", self.source_harness.as_str()),
            ("targetHarness", self.target_harness.as_str()),
            ("permissionMode", self.permission_mode.as_str()),
            ("outputContract", self.output_contract.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{field} is required"));
            }
        }
        if self.evidence_ids.iter().any(|id| id.trim().is_empty()) {
            return Err("evidenceIds cannot contain empty values".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_packet_requires_portable_handoff_fields() {
        let packet = HandoffPacket {
            schema_version: 1,
            trace_id: "a".into(),
            source_harness: "codex".into(),
            target_harness: "claude".into(),
            repository_revision: Some("abc".into()),
            permission_mode: "workspace-write".into(),
            budget: serde_json::json!({"tokens": 10}),
            evidence_ids: vec!["e1".into()],
            parent_entry_id: Some("entry".into()),
            output_contract: "implementation-result".into(),
        };
        assert!(packet.validate().is_ok());
        assert!(HandoffPacket {
            schema_version: 2,
            ..packet
        }
        .validate()
        .is_err());
    }
}
