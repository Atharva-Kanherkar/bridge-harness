//! Deterministic prompt assembly for provider cache reuse.
//!
//! The stable prefix is canonical, versioned, and deliberately separate from
//! per-task facts and session capabilities. Provider adapters receive the
//! rendered prompt unchanged so Codex and Claude see the stable bytes first.

use crate::{secret_interception, BridgeError};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const PROMPT_SCHEMA_VERSION: u32 = 1;
pub const MAX_VARIABLE_SUFFIX_BYTES: usize = 128 * 1024;
pub const TOKEN_ESTIMATE_SOURCE: &str = "bytes_div4_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMetadata {
    pub schema_version: u32,
    pub prefix_id: String,
    pub prefix_hash: String,
    pub prefix_bytes: usize,
    pub prefix_token_estimate: u64,
}

/// Which region of the wire payload (`stable_prefix + "\n\n" + variable_suffix`)
/// an accounting entry's bytes belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptAccountingRegion {
    Stable,
    Variable,
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptAccountingKind {
    Role,
    StableSection,
    ToolSchema,
    ProjectRule,
    VariableSection,
    Overhead,
}

/// A leave-one-out attribution of serialized bytes to one causing element
/// (or, for `Overhead`, to envelope tags/punctuation charged to no element).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptAccountingEntry {
    pub region: PromptAccountingRegion,
    pub kind: PromptAccountingKind,
    pub name: String,
    pub bytes: usize,
    pub token_estimate: u64,
}

impl PromptAccountingEntry {
    fn new(
        region: PromptAccountingRegion,
        kind: PromptAccountingKind,
        name: impl Into<String>,
        bytes: usize,
    ) -> Self {
        Self {
            region,
            kind,
            name: name.into(),
            bytes,
            token_estimate: bytes.div_ceil(4) as u64,
        }
    }
}

/// Exact byte accounting for a compiled prompt. Entries sum to
/// `stable_bytes + variable_bytes + 2` (the `\n\n` separator) with zero
/// unattributed remainder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptAccounting {
    pub entries: Vec<PromptAccountingEntry>,
    pub stable_bytes: usize,
    pub variable_bytes: usize,
    pub stable_token_estimate: u64,
    pub variable_token_estimate: u64,
    pub token_estimate_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPrompt {
    pub metadata: PromptMetadata,
    pub stable_prefix: String,
    pub variable_suffix: String,
    pub accounting: PromptAccounting,
    instructions: String,
}

impl CompiledPrompt {
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptCompiler {
    role: String,
    stable_sections: BTreeMap<String, String>,
    tool_schemas: BTreeMap<String, Value>,
    project_rules: BTreeMap<String, String>,
    variable_sections: Vec<NamedText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct NamedText {
    name: String,
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StableEnvelope<'a> {
    schema_version: u32,
    // `Option` so leave-one-out accounting can serialize a "role removed"
    // variant by passing `None`; real compilations always pass `Some`, so
    // the wire bytes are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
    stable_sections: &'a BTreeMap<String, String>,
    tool_schemas: &'a BTreeMap<String, Value>,
    project_rules: &'a BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VariableEnvelope<'a> {
    sections: &'a [NamedText],
}

impl PromptCompiler {
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            role: canonical_text(&role.into()),
            ..Self::default()
        }
    }

    pub fn stable_section(mut self, name: impl Into<String>, text: impl Into<String>) -> Self {
        insert_text(&mut self.stable_sections, name, text);
        self
    }

    pub fn tool_schema(mut self, name: impl Into<String>, schema: Value) -> Self {
        self.tool_schemas
            .insert(canonical_text(&name.into()), canonical_json(schema));
        self
    }

    pub fn project_rule(mut self, name: impl Into<String>, text: impl Into<String>) -> Self {
        insert_text(&mut self.project_rules, name, text);
        self
    }

    pub fn variable_section(mut self, name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = canonical_text(&text.into());
        if !text.is_empty() {
            self.variable_sections.push(NamedText {
                name: canonical_text(&name.into()),
                text,
            });
        }
        self
    }

    pub fn compile(self) -> Result<CompiledPrompt, BridgeError> {
        if self.role.is_empty() {
            return Err(BridgeError::Invalid(
                "Prompt compiler role must not be empty".into(),
            ));
        }
        validate_stable_value("role", &self.role)?;
        for (name, text) in &self.stable_sections {
            validate_stable_value(name, text)?;
        }
        for (name, schema) in &self.tool_schemas {
            validate_stable_value(name, &serde_json::to_string(schema).map_err(prompt_error)?)?;
        }
        for (name, text) in &self.project_rules {
            validate_stable_value(name, text)?;
        }

        let stable_json = serde_json::to_string(&StableEnvelope {
            schema_version: PROMPT_SCHEMA_VERSION,
            role: Some(self.role.as_str()),
            stable_sections: &self.stable_sections,
            tool_schemas: &self.tool_schemas,
            project_rules: &self.project_rules,
        })
        .map_err(prompt_error)?;
        let stable_prefix = format!(
            "<bridge-stable-prompt schema=\"{PROMPT_SCHEMA_VERSION}\">\n{stable_json}\n</bridge-stable-prompt>"
        );
        let variable_json = serde_json::to_string(&VariableEnvelope {
            sections: &self.variable_sections,
        })
        .map_err(prompt_error)?;
        let variable_suffix =
            format!("<bridge-variable-context>\n{variable_json}\n</bridge-variable-context>");
        if variable_suffix.len() > MAX_VARIABLE_SUFFIX_BYTES {
            return Err(BridgeError::Invalid(format!(
                "Variable prompt context is {} bytes; maximum is {MAX_VARIABLE_SUFFIX_BYTES}",
                variable_suffix.len()
            )));
        }

        let prefix_hash = sha256_hex(stable_prefix.as_bytes());
        let prefix_id = format!(
            "bridge-prompt-v{PROMPT_SCHEMA_VERSION}-{}",
            &prefix_hash[..16]
        );
        let prefix_bytes = stable_prefix.len();
        let metadata = PromptMetadata {
            schema_version: PROMPT_SCHEMA_VERSION,
            prefix_id,
            prefix_hash,
            prefix_bytes,
            prefix_token_estimate: prefix_bytes.div_ceil(4) as u64,
        };
        let instructions = format!("{stable_prefix}\n\n{variable_suffix}");
        let accounting = compute_accounting(
            &self.role,
            &self.stable_sections,
            &self.tool_schemas,
            &self.project_rules,
            &self.variable_sections,
            stable_json.len(),
            variable_json.len(),
            stable_prefix.len(),
            variable_suffix.len(),
        )?;
        Ok(CompiledPrompt {
            metadata,
            stable_prefix,
            variable_suffix,
            accounting,
            instructions,
        })
    }
}

/// The one way a resolved prompt stack becomes a compiler: every surviving
/// section enters, in resolve order, as a stable section. Both the live turn
/// path and the Prompt Studio preview must use this builder so their
/// compositions cannot drift.
pub fn compiler_for_resolved_stack(
    stack: &crate::prompt_sections::ResolvedPromptStack,
) -> Result<PromptCompiler, BridgeError> {
    let mut compiler = PromptCompiler::new(stack.target.compiler_role());
    for section in &stack.sections {
        compiler = compiler.stable_section(&section.id, &section.text);
    }
    Ok(compiler)
}

/// Leave-one-out delta serialization: for each surviving element, serialize
/// the envelope once with it present and once with it removed; the byte
/// difference is charged to that element (escaping, quotes, colons, and
/// separators included). What's left over — tag literals, braces, the
/// `schema_version` field, and any un-attributed JSON punctuation — becomes
/// an explicit overhead entry so the region's bytes close exactly.
#[allow(clippy::too_many_arguments)]
fn compute_accounting(
    role: &str,
    stable_sections: &BTreeMap<String, String>,
    tool_schemas: &BTreeMap<String, Value>,
    project_rules: &BTreeMap<String, String>,
    variable_sections: &[NamedText],
    stable_json_len: usize,
    variable_json_len: usize,
    stable_prefix_len: usize,
    variable_suffix_len: usize,
) -> Result<PromptAccounting, BridgeError> {
    let mut entries = Vec::new();
    let mut stable_element_total = 0usize;

    let role_removed_len = serde_json::to_string(&StableEnvelope {
        schema_version: PROMPT_SCHEMA_VERSION,
        role: None,
        stable_sections,
        tool_schemas,
        project_rules,
    })
    .map_err(prompt_error)?
    .len();
    let role_bytes = stable_json_len - role_removed_len;
    stable_element_total += role_bytes;
    entries.push(PromptAccountingEntry::new(
        PromptAccountingRegion::Stable,
        PromptAccountingKind::Role,
        "role",
        role_bytes,
    ));

    for name in stable_sections.keys() {
        let mut without = stable_sections.clone();
        without.remove(name);
        let without_len = serde_json::to_string(&StableEnvelope {
            schema_version: PROMPT_SCHEMA_VERSION,
            role: Some(role),
            stable_sections: &without,
            tool_schemas,
            project_rules,
        })
        .map_err(prompt_error)?
        .len();
        let bytes = stable_json_len - without_len;
        stable_element_total += bytes;
        entries.push(PromptAccountingEntry::new(
            PromptAccountingRegion::Stable,
            PromptAccountingKind::StableSection,
            name.clone(),
            bytes,
        ));
    }

    for name in tool_schemas.keys() {
        let mut without = tool_schemas.clone();
        without.remove(name);
        let without_len = serde_json::to_string(&StableEnvelope {
            schema_version: PROMPT_SCHEMA_VERSION,
            role: Some(role),
            stable_sections,
            tool_schemas: &without,
            project_rules,
        })
        .map_err(prompt_error)?
        .len();
        let bytes = stable_json_len - without_len;
        stable_element_total += bytes;
        entries.push(PromptAccountingEntry::new(
            PromptAccountingRegion::Stable,
            PromptAccountingKind::ToolSchema,
            name.clone(),
            bytes,
        ));
    }

    for name in project_rules.keys() {
        let mut without = project_rules.clone();
        without.remove(name);
        let without_len = serde_json::to_string(&StableEnvelope {
            schema_version: PROMPT_SCHEMA_VERSION,
            role: Some(role),
            stable_sections,
            tool_schemas,
            project_rules: &without,
        })
        .map_err(prompt_error)?
        .len();
        let bytes = stable_json_len - without_len;
        stable_element_total += bytes;
        entries.push(PromptAccountingEntry::new(
            PromptAccountingRegion::Stable,
            PromptAccountingKind::ProjectRule,
            name.clone(),
            bytes,
        ));
    }

    let stable_overhead_bytes = stable_prefix_len - stable_element_total;
    entries.push(PromptAccountingEntry::new(
        PromptAccountingRegion::Stable,
        PromptAccountingKind::Overhead,
        "stable_envelope",
        stable_overhead_bytes,
    ));

    let mut variable_element_total = 0usize;
    for index in 0..variable_sections.len() {
        let mut without = variable_sections.to_vec();
        without.remove(index);
        let without_len = serde_json::to_string(&VariableEnvelope { sections: &without })
            .map_err(prompt_error)?
            .len();
        let bytes = variable_json_len - without_len;
        variable_element_total += bytes;
        entries.push(PromptAccountingEntry::new(
            PromptAccountingRegion::Variable,
            PromptAccountingKind::VariableSection,
            variable_sections[index].name.clone(),
            bytes,
        ));
    }

    let variable_overhead_bytes = variable_suffix_len - variable_element_total;
    entries.push(PromptAccountingEntry::new(
        PromptAccountingRegion::Variable,
        PromptAccountingKind::Overhead,
        "variable_envelope",
        variable_overhead_bytes,
    ));

    entries.push(PromptAccountingEntry::new(
        PromptAccountingRegion::Separator,
        PromptAccountingKind::Overhead,
        "separator",
        2,
    ));

    Ok(PromptAccounting {
        entries,
        stable_bytes: stable_prefix_len,
        variable_bytes: variable_suffix_len,
        stable_token_estimate: stable_prefix_len.div_ceil(4) as u64,
        variable_token_estimate: variable_suffix_len.div_ceil(4) as u64,
        token_estimate_source: TOKEN_ESTIMATE_SOURCE.to_string(),
    })
}

fn insert_text(
    target: &mut BTreeMap<String, String>,
    name: impl Into<String>,
    text: impl Into<String>,
) {    let name = canonical_text(&name.into());
    let text = canonical_text(&text.into());
    if !text.is_empty() {
        target.insert(name, text);
    }
}

fn canonical_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned()
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn validate_stable_value(name: &str, value: &str) -> Result<(), BridgeError> {
    let sanitized = secret_interception::sanitize(value);
    let lower = value.to_ascii_lowercase();
    let contains_capability = lower.contains("[secret:")
        || lower.contains("/credential-proxy/")
        || lower.contains("x-bridge-proxy-auth");
    if !sanitized.interceptions.is_empty() || contains_capability {
        return Err(BridgeError::Invalid(format!(
            "Stable prompt field `{name}` contains secret or session-capability material"
        )));
    }
    Ok(())
}

fn sha256_hex(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn prompt_error(error: serde_json::Error) -> BridgeError {
    BridgeError::Invalid(format!("Could not compile prompt: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compiler(variable: &str) -> PromptCompiler {
        PromptCompiler::new("worker:implementation")
            .stable_section("role", "Implement one focused change.")
            .tool_schema(
                "zeta",
                json!({"required":["path"],"properties":{"path":{"type":"string"}},"type":"object"}),
            )
            .tool_schema("alpha", json!({"type":"object"}))
            .project_rule("repository", "Use Tailwind CSS v4.")
            .variable_section("task", variable)
    }

    #[test]
    fn identical_invariants_produce_byte_stable_prefixes() {
        let first = compiler("Task one").compile().unwrap();
        let second = compiler("Task one").compile().unwrap();
        assert_eq!(
            first.stable_prefix.as_bytes(),
            second.stable_prefix.as_bytes()
        );
        assert_eq!(first.metadata, second.metadata);
        assert!(first.instructions().starts_with(&first.stable_prefix));
        assert!(
            first.stable_prefix.contains("\"alpha\"") && first.stable_prefix.contains("\"zeta\"")
        );
        assert!(
            first.stable_prefix.find("\"alpha\"").unwrap()
                < first.stable_prefix.find("\"zeta\"").unwrap()
        );
    }

    #[test]
    fn tool_schema_object_keys_are_canonicalized() {
        let mut first_schema = serde_json::Map::new();
        first_schema.insert("zeta".into(), json!({"type":"string"}));
        first_schema.insert("alpha".into(), json!({"type":"number"}));
        let mut second_schema = serde_json::Map::new();
        second_schema.insert("alpha".into(), json!({"type":"number"}));
        second_schema.insert("zeta".into(), json!({"type":"string"}));

        let first = PromptCompiler::new("worker")
            .tool_schema("tool", Value::Object(first_schema))
            .compile()
            .unwrap();
        let second = PromptCompiler::new("worker")
            .tool_schema("tool", Value::Object(second_schema))
            .compile()
            .unwrap();
        assert_eq!(first.metadata.prefix_hash, second.metadata.prefix_hash);
        assert_eq!(first.stable_prefix, second.stable_prefix);
    }

    #[test]
    fn variable_context_does_not_change_the_prefix_identity() {
        let first = compiler("Task one at 2026-07-21").compile().unwrap();
        let second = compiler("Task two at 2026-07-22").compile().unwrap();
        assert_eq!(first.metadata.prefix_hash, second.metadata.prefix_hash);
        assert_eq!(first.metadata.prefix_id, second.metadata.prefix_id);
        assert_ne!(first.variable_suffix, second.variable_suffix);
    }

    #[test]
    fn role_tool_and_rule_changes_invalidate_the_prefix() {
        let base = compiler("task").compile().unwrap();
        let role = PromptCompiler::new("worker:review")
            .stable_section("role", "Implement one focused change.")
            .compile()
            .unwrap();
        let tool = compiler("task")
            .tool_schema("alpha", json!({"type":"string"}))
            .compile()
            .unwrap();
        let rule = compiler("task")
            .project_rule("repository", "Use plain CSS.")
            .compile()
            .unwrap();
        for changed in [role, tool, rule] {
            assert_ne!(base.metadata.prefix_hash, changed.metadata.prefix_hash);
        }
    }

    #[test]
    fn stable_prefix_rejects_secrets_and_session_capabilities() {
        let secret = PromptCompiler::new("worker")
            .stable_section(
                "unsafe",
                "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz123456",
            )
            .compile()
            .unwrap_err();
        assert!(secret.to_string().contains("secret or session-capability"));

        let capability = PromptCompiler::new("worker")
            .project_rule(
                "unsafe",
                "Call /credential-proxy/session/reference with x-bridge-proxy-auth.",
            )
            .compile()
            .unwrap_err();
        assert!(capability
            .to_string()
            .contains("secret or session-capability"));
    }

    #[test]
    fn variable_suffix_is_bounded_without_affecting_valid_capability_context() {
        let allowed = compiler("Use [secret:sec_example] through /credential-proxy/session/ref")
            .compile()
            .unwrap();
        assert!(allowed.variable_suffix.contains("credential-proxy"));

        let error = compiler(&"x".repeat(MAX_VARIABLE_SUFFIX_BYTES))
            .compile()
            .unwrap_err();
        assert!(error.to_string().contains("maximum"));
    }

    #[test]
    fn accounting_closes_to_the_exact_wire_bytes() {
        let compiled = compiler("Task one").compile().unwrap();
        let accounting = &compiled.accounting;

        let stable_total: usize = accounting
            .entries
            .iter()
            .filter(|entry| entry.region == PromptAccountingRegion::Stable)
            .map(|entry| entry.bytes)
            .sum();
        assert_eq!(stable_total, compiled.stable_prefix.len());

        let variable_total: usize = accounting
            .entries
            .iter()
            .filter(|entry| entry.region == PromptAccountingRegion::Variable)
            .map(|entry| entry.bytes)
            .sum();
        assert_eq!(variable_total, compiled.variable_suffix.len());

        let separator_bytes: usize = accounting
            .entries
            .iter()
            .filter(|entry| entry.region == PromptAccountingRegion::Separator)
            .map(|entry| entry.bytes)
            .sum();
        assert_eq!(separator_bytes, 2);

        let grand_total: usize = accounting.entries.iter().map(|entry| entry.bytes).sum();
        assert_eq!(grand_total, compiled.instructions().len());
    }

    #[test]
    fn json_escaping_is_charged_to_the_causing_element() {
        let text = "line one\"quote\\backslash\nline two";
        let compiled = PromptCompiler::new("worker")
            .stable_section("weird", text)
            .compile()
            .unwrap();
        let entry = compiled
            .accounting
            .entries
            .iter()
            .find(|entry| entry.kind == PromptAccountingKind::StableSection && entry.name == "weird")
            .unwrap();
        assert!(entry.bytes > text.len());
    }

    #[test]
    fn empty_and_deleted_sections_produce_no_phantom_entries() {
        let compiled = PromptCompiler::new("worker")
            .stable_section("kept", "")
            .project_rule("kept", "")
            .variable_section("kept", "")
            .compile()
            .unwrap();
        assert!(compiled
            .accounting
            .entries
            .iter()
            .all(|entry| entry.kind != PromptAccountingKind::StableSection
                && entry.kind != PromptAccountingKind::ProjectRule
                && entry.kind != PromptAccountingKind::VariableSection));
    }

    #[test]
    fn separator_and_tags_are_explicitly_attributed() {
        let compiled = compiler("Task one").compile().unwrap();
        assert!(compiled.accounting.entries.iter().any(|entry| entry.region
            == PromptAccountingRegion::Stable
            && entry.kind == PromptAccountingKind::Overhead
            && entry.name == "stable_envelope"));
        assert!(compiled.accounting.entries.iter().any(|entry| entry.region
            == PromptAccountingRegion::Variable
            && entry.kind == PromptAccountingKind::Overhead
            && entry.name == "variable_envelope"));
        assert!(compiled.accounting.entries.iter().any(|entry| entry.region
            == PromptAccountingRegion::Separator
            && entry.name == "separator"
            && entry.bytes == 2));
        for entry in &compiled.accounting.entries {
            assert!(entry.bytes < compiled.instructions().len());
        }
    }

    #[test]
    fn token_estimates_are_labelled_and_bounded() {
        let compiled = compiler("Task one").compile().unwrap();
        let accounting = &compiled.accounting;
        assert_eq!(accounting.token_estimate_source, TOKEN_ESTIMATE_SOURCE);
        assert_eq!(
            accounting.stable_token_estimate,
            accounting.stable_bytes.div_ceil(4) as u64
        );
        assert_eq!(
            accounting.variable_token_estimate,
            accounting.variable_bytes.div_ceil(4) as u64
        );
        assert!(accounting.variable_bytes <= MAX_VARIABLE_SUFFIX_BYTES);
        assert!(accounting.variable_token_estimate <= MAX_VARIABLE_SUFFIX_BYTES.div_ceil(4) as u64);
    }

    #[test]
    fn prefix_identity_is_unchanged_by_accounting() {
        let first = compiler("Task one").compile().unwrap();
        let second = compiler("Task one").compile().unwrap();
        assert_eq!(first.metadata, second.metadata);
        assert_eq!(first.stable_prefix, second.stable_prefix);
        assert_eq!(first.metadata.prefix_bytes, first.stable_prefix.len());
        assert!(!first.stable_prefix.contains("tokenEstimate"));
        assert!(!first.variable_suffix.contains("tokenEstimate"));
    }
}
