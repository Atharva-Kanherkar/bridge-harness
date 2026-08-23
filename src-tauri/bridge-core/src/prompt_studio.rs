//! The Prompt Studio protocol surface (#241): reading one target's resolved
//! prompt stack, mutating sections through the 1/8 revision store, and
//! compiling the exact Bridge-authored preview envelopes.
//!
//! Preview honesty is the load-bearing rule here. The stable prefix and the
//! variable suffix in [`CompiledPromptPreview`] are the exact bytes Bridge's
//! own compiler produces for the resolved section stack under an empty
//! variable context — nothing else may claim exactness. Provider-owned
//! material is described by [`PromptProviderLayerStatus`] rows carrying a
//! closed source vocabulary (`reported | measured | estimated | unavailable`)
//! and never carries invented bytes: until an adapter capture exists, every
//! provider-base row is `unavailable` with the authority reason attached.

use crate::{
    delegation, prompt_authority, prompt_compiler, prompt_sections,
    prompt_sections::PromptSectionOperation, prompts, BridgeError,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// The most recent window of each section's history embedded in views. The
/// store keeps everything append-only; views stay bounded so responses cannot
/// grow without limit. Older revisions remain addressable by id for restore.
pub const MAX_REVISIONS_IN_VIEW: usize = 50;

/// How a provider-owned layer's `bytes` value was obtained. Closed vocabulary:
/// never fabricate a number that a source below does not defend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptLayerSource {
    /// The provider reported the number itself.
    Reported,
    /// Bridge observed the exact bytes.
    Measured,
    /// A labelled approximation.
    Estimated,
    /// Nothing defensible exists.
    Unavailable,
}

/// One section of a target's prompt stack, as the studio sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSectionView {
    pub id: String,
    pub state: prompt_sections::PromptSectionState,
    /// The built-in text this section falls back to when its state is default.
    pub default_text: String,
    /// The text the live compiler resolves to, or `None` when the user deleted
    /// the section — deletion is a real state, not whitespace.
    pub effective_text: Option<String>,
    pub bytes: u64,
    /// Byte-derived estimate (`bytes.div_ceil(4)`), labelled as an estimate.
    pub token_estimate: u64,
    pub lint_warnings: Vec<PromptLintWarningView>,
    /// Append-only history, oldest first.
    pub revisions: Vec<PromptRevisionView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStackView {
    /// Storage key of the target (`orchestrator`, `worker:<role>`,
    /// `direct_session`) — the same vocabulary the params enum accepts.
    pub target: String,
    /// Worker topology depth the stack was resolved at.
    pub depth: i64,
    pub sections: Vec<PromptSectionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRevisionView {
    pub id: i64,
    pub operation: PromptSectionOperation,
    pub state: prompt_sections::PromptSectionState,
    pub restored_from_revision_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptLintWarningView {
    pub marker: String,
    pub message: String,
}

/// What a mutation changed: the revision it appended plus the fresh stack for
/// the target, so a client never mutates against a stale view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSectionMutation {
    pub revision: PromptRevisionView,
    pub stack: PromptStackView,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledPromptPreview {
    pub target: String,
    pub depth: i64,
    pub stack: PromptStackView,
    /// Exact `<bridge-stable-prompt …>` bytes compiled from the resolved
    /// sections alone — no project rules, tool schemas, or runtime variables.
    pub stable_prefix: String,
    /// Exact `<bridge-variable-context>` bytes with an empty sections list.
    /// Real task/session/restoration/memory content is runtime data and is
    /// deliberately not fabricated here.
    pub variable_suffix: String,
    pub schema_version: u32,
    pub prefix_id: String,
    pub prefix_hash: String,
    pub prefix_bytes: u64,
    pub prefix_token_estimate: u64,
    pub provider_layers: Vec<PromptProviderLayerStatus>,
}

/// One provider-owned layer's honest standing for the preview.
///
/// `source` is a closed vocabulary describing how `bytes` was obtained:
/// `reported` (the provider told us), `measured` (Bridge observed the bytes),
/// `estimated` (a labelled approximation), `unavailable` (nothing defensible).
/// Until an adapter-capture slice exists, every provider-base row is
/// `unavailable` — the authority matrix decides, never convenience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptProviderLayerStatus {
    pub layer: String,
    pub adapter: String,
    pub source: PromptLayerSource,
    /// Exact byte size when actually readable; never invented.
    pub bytes: Option<u64>,
    pub detail: Option<String>,
}

fn validate_depth(depth: i64) -> Result<(), BridgeError> {
    if !(0..=delegation::DEFAULT_MAX_DEPTH).contains(&depth) {
        return Err(BridgeError::Invalid(format!(
            "worker depth {depth} is outside the supported range 0..={}",
            delegation::DEFAULT_MAX_DEPTH
        )));
    }
    Ok(())
}

fn storage_key(target: prompts::PromptTarget) -> String {
    target.storage_key().to_owned()
}

fn revision_view(revision: &prompt_sections::PromptSectionRevision) -> PromptRevisionView {
    PromptRevisionView {
        id: revision.id,
        operation: revision.operation,
        state: revision.state.clone(),
        restored_from_revision_id: revision.restored_from_revision_id,
        created_at: revision.created_at.clone(),
    }
}

/// The bounded history window: the newest [`MAX_REVISIONS_IN_VIEW`] revisions,
/// kept oldest-first so clients render chronological order without reversal.
fn bounded_revisions(
    db: &Connection,
    key: &prompt_sections::PromptSectionKey,
) -> Result<Vec<PromptRevisionView>, BridgeError> {
    let mut revisions = prompt_sections::revisions(db, key)?
        .iter()
        .map(revision_view)
        .collect::<Vec<_>>();
    if revisions.len() > MAX_REVISIONS_IN_VIEW {
        revisions.drain(..revisions.len() - MAX_REVISIONS_IN_VIEW);
    }
    Ok(revisions)
}

fn section_view(
    db: &Connection,
    target: prompts::PromptTarget,
    default_section: &prompts::PromptDefaultSection,
) -> Result<PromptSectionView, BridgeError> {
    let key = prompt_sections::PromptSectionKey::new(target, default_section.id)?;
    let state = prompt_sections::current_state(db, &key)?;
    let effective_text = match &state {
        prompt_sections::PromptSectionState::Deleted => None,
        prompt_sections::PromptSectionState::Default => Some(default_section.text.clone()),
        prompt_sections::PromptSectionState::Overridden { text } => Some(text.clone()),
    };
    let bytes = effective_text.as_ref().map_or(0, |text| text.len()) as u64;
    let lint_warnings = effective_text
        .as_deref()
        .map(prompts::lint_required_markers)
        .unwrap_or_default()
        .into_iter()
        .map(|warning| PromptLintWarningView {
            marker: warning.marker.to_owned(),
            message: warning.message,
        })
        .collect();
    let revisions = bounded_revisions(db, &key)?;
    Ok(PromptSectionView {
        id: default_section.id.to_owned(),
        state,
        default_text: default_section.text.clone(),
        effective_text,
        bytes,
        token_estimate: bytes.div_ceil(4),
        lint_warnings,
        revisions,
    })
}

fn stack_view(
    db: &Connection,
    target: prompts::PromptTarget,
    depth: i64,
) -> Result<PromptStackView, BridgeError> {
    validate_depth(depth)?;
    let defaults = prompts::default_sections(target, depth);
    let sections = defaults
        .iter()
        .map(|default| section_view(db, target, default))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PromptStackView {
        target: storage_key(target),
        depth,
        sections,
    })
}

/// The resolved prompt stack for one target: states, defaults, sizes, lint,
/// and the append-only revision history per section.
pub fn stack(
    db: &Connection,
    target: prompts::PromptTarget,
    depth: i64,
) -> Result<PromptStackView, BridgeError> {
    stack_view(db, target, depth)
}

pub fn save_section(
    db: &Connection,
    target: prompts::PromptTarget,
    section_id: &str,
    depth: i64,
    text: &str,
) -> Result<PromptSectionMutation, BridgeError> {
    // Validate everything before the first write: a rejected request must
    // leave states, texts, and revision counts exactly as they were.
    validate_depth(depth)?;
    let key = prompt_sections::PromptSectionKey::new(target, section_id)?;
    let revision = prompt_sections::save_override(db, &key, text)?;
    Ok(PromptSectionMutation {
        revision: revision_view(&revision),
        stack: stack_view(db, target, depth)?,
    })
}

pub fn reset_section(
    db: &Connection,
    target: prompts::PromptTarget,
    section_id: &str,
    depth: i64,
) -> Result<PromptSectionMutation, BridgeError> {
    validate_depth(depth)?;
    let key = prompt_sections::PromptSectionKey::new(target, section_id)?;
    let revision = prompt_sections::reset_section(db, &key)?;
    Ok(PromptSectionMutation {
        revision: revision_view(&revision),
        stack: stack_view(db, target, depth)?,
    })
}

pub fn restore_revision(
    db: &Connection,
    target: prompts::PromptTarget,
    section_id: &str,
    revision_id: i64,
    depth: i64,
) -> Result<PromptSectionMutation, BridgeError> {
    validate_depth(depth)?;
    let key = prompt_sections::PromptSectionKey::new(target, section_id)?;
    let revision = prompt_sections::restore_revision(db, &key, revision_id)?;
    Ok(PromptSectionMutation {
        revision: revision_view(&revision),
        stack: stack_view(db, target, depth)?,
    })
}

fn provider_layer_statuses() -> Vec<PromptProviderLayerStatus> {
    prompt_authority::provider_base_prompt_authorities()
        .iter()
        .map(|capability| {
            // No adapter capture exists yet, so no provider-base row may claim
            // bytes. A future capture slice flips rows to measured/reported;
            // the match below is exactly what it must extend.
            let (source, detail) = match capability.readable {
                prompt_authority::PromptVerdict::Unsupported { reason } => {
                    (PromptLayerSource::Unavailable, Some(reason.to_string()))
                }
                prompt_authority::PromptVerdict::Supported => (
                    PromptLayerSource::Unavailable,
                    Some(
                        "this adapter could expose its base prompt, but Bridge does not \
                         capture provider-base bytes yet"
                            .to_string(),
                    ),
                ),
            };
            PromptProviderLayerStatus {
                layer: "provider_base".to_owned(),
                adapter: capability.adapter.to_owned(),
                source,
                bytes: None,
                detail,
            }
        })
        .collect()
}

/// The exact Bridge-authored envelopes for one target's resolved stack.
///
/// The preview compiles the resolved sections alone: overrides applied,
/// deletions omitted, no configured project rules, tool schemas, or runtime
/// variable content. Both envelope strings are byte-exact for that condition,
/// and the hash/id/size metadata describe exactly those bytes. Because
/// runtime variable sections never move the prefix hash, a live turn built
/// from this same section stack and no configured project rules shares this
/// preview's prefix hash; any other live turn hashes different stable inputs.
pub fn preview(
    db: &Connection,
    target: prompts::PromptTarget,
    depth: i64,
) -> Result<CompiledPromptPreview, BridgeError> {
    let stack = stack_view(db, target, depth)?;
    let resolved = prompt_sections::resolve(db, target, depth)?;
    // The exact same builder the live turn path uses — one composition, no drift.
    let compiled = prompt_compiler::compiler_for_resolved_stack(&resolved)?.compile()?;
    Ok(CompiledPromptPreview {
        target: stack.target.clone(),
        depth,
        stack,
        schema_version: compiled.metadata.schema_version,
        prefix_id: compiled.metadata.prefix_id.clone(),
        prefix_hash: compiled.metadata.prefix_hash.clone(),
        prefix_bytes: compiled.metadata.prefix_bytes as u64,
        prefix_token_estimate: compiled.metadata.prefix_token_estimate,
        stable_prefix: compiled.stable_prefix,
        variable_suffix: compiled.variable_suffix,
        provider_layers: provider_layer_statuses(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent_config, store};
    use std::path::Path;

    fn db() -> Connection {
        store::open(Path::new(":memory:")).unwrap()
    }

    fn section<'a>(stack: &'a PromptStackView, id: &str) -> &'a PromptSectionView {
        stack
            .sections
            .iter()
            .find(|section| section.id == id)
            .unwrap_or_else(|| panic!("stack is missing section {id}"))
    }

    #[test]
    fn stack_reports_states_defaults_sizes_lint_and_revisions() {
        let db = db();
        let orchestrator = stack(&db, prompts::PromptTarget::Orchestrator, 0).unwrap();
        assert_eq!(orchestrator.target, "orchestrator");
        assert_eq!(
            orchestrator
                .sections
                .iter()
                .map(|section| section.id.as_str())
                .collect::<Vec<_>>(),
            [prompts::BRIDGE_ROLE_SECTION_ID, prompts::DELEGATION_PROTOCOL_SECTION_ID]
        );
        let role = section(&orchestrator, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(role.state, prompt_sections::PromptSectionState::Default);
        assert_eq!(role.effective_text.as_deref(), Some(role.default_text.as_str()));
        assert_eq!(role.bytes, role.default_text.len() as u64);
        assert_eq!(role.token_estimate, role.bytes.div_ceil(4));
        assert!(role.lint_warnings.is_empty(), "defaults satisfy their own markers");
        assert!(role.revisions.is_empty());

        prompt_sections::save_override(
            &db,
            &prompt_sections::PromptSectionKey::new(
                prompts::PromptTarget::Orchestrator,
                prompts::BRIDGE_ROLE_SECTION_ID,
            )
            .unwrap(),
            "A custom policy that drops the delegation vocabulary.",
        )
        .unwrap();
        let edited = stack(&db, prompts::PromptTarget::Orchestrator, 0).unwrap();
        let role = section(&edited, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(
            role.state,
            prompt_sections::PromptSectionState::Overridden {
                text: "A custom policy that drops the delegation vocabulary.".into()
            }
        );
        assert_eq!(role.bytes, role.effective_text.as_ref().unwrap().len() as u64);
        assert!(
            !role.lint_warnings.is_empty(),
            "an edit that strips required markers must surface lint warnings"
        );
        assert_eq!(role.revisions.len(), 1);
        assert_eq!(role.revisions[0].operation, PromptSectionOperation::Override);

        prompt_sections::delete_section(
            &db,
            &prompt_sections::PromptSectionKey::new(
                prompts::PromptTarget::Orchestrator,
                prompts::BRIDGE_ROLE_SECTION_ID,
            )
            .unwrap(),
        )
        .unwrap();
        let deleted_stack = stack(&db, prompts::PromptTarget::Orchestrator, 0).unwrap();
        let role = section(&deleted_stack, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(role.state, prompt_sections::PromptSectionState::Deleted);
        assert_eq!(role.effective_text, None);
        assert_eq!(role.bytes, 0);
        assert_eq!(role.token_estimate, 0);
        assert!(role.lint_warnings.is_empty(), "deleted sections have nothing to lint");

        // Direct sessions carry no Bridge-authored stable sections at all.
        let direct = stack(&db, prompts::PromptTarget::DirectSession, 0).unwrap();
        assert_eq!(direct.target, "direct_session");
        assert!(direct.sections.is_empty());
    }

    #[test]
    fn mutations_return_the_appended_revision_and_fresh_stack() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;

        let saved =
            save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Policy v1").unwrap();
        assert_eq!(saved.revision.operation, PromptSectionOperation::Override);
        assert_eq!(saved.stack.target, "orchestrator");
        let saved_role = section(&saved.stack, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(saved_role.effective_text.as_deref(), Some("Policy v1"));
        assert_eq!(saved_role.revisions.len(), 1);
        assert_eq!(saved_role.revisions[0].id, saved.revision.id);

        // Re-saving identical text stays deterministic: another override
        // revision appends; history is never rewritten or deduplicated away.
        let again =
            save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Policy v1").unwrap();
        assert!(again.revision.id > saved.revision.id);
        assert_eq!(
            section(&again.stack, prompts::BRIDGE_ROLE_SECTION_ID).revisions.len(),
            2
        );

        let reset = reset_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0).unwrap();
        assert_eq!(reset.revision.operation, PromptSectionOperation::Reset);
        let reset_role = section(&reset.stack, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(reset_role.state, prompt_sections::PromptSectionState::Default);
        assert_eq!(reset_role.revisions.len(), 3);

        let restored =
            restore_revision(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, saved.revision.id, 0)
                .unwrap();
        assert_eq!(restored.revision.operation, PromptSectionOperation::Restore);
        assert_eq!(restored.revision.restored_from_revision_id, Some(saved.revision.id));
        let restored_role = section(&restored.stack, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(restored_role.effective_text.as_deref(), Some("Policy v1"));
        assert_eq!(restored_role.revisions.len(), 4);

        // The returned stack always agrees with a fresh read of the database.
        let fresh = stack(&db, target, 0).unwrap();
        assert_eq!(fresh, restored.stack);
    }

    #[test]
    fn restore_rejects_revisions_from_other_sections() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        let saved =
            save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Role policy").unwrap();
        let error = restore_revision(
            &db,
            target,
            prompts::DELEGATION_PROTOCOL_SECTION_ID,
            saved.revision.id,
            0,
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not belong"));
        // The failed restore must not have touched the other section.
        assert!(section(
            &stack(&db, target, 0).unwrap(),
            prompts::DELEGATION_PROTOCOL_SECTION_ID
        )
        .revisions
        .is_empty());
    }

    #[test]
    fn failed_depth_mutations_have_no_side_effects() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Before.").unwrap();
        let before = stack(&db, target, 0).unwrap();

        for bad_depth in [-1, delegation::DEFAULT_MAX_DEPTH + 1] {
            assert!(save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, bad_depth, "x")
                .is_err());
            assert!(reset_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, bad_depth).is_err());
            assert!(
                restore_revision(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 1, bad_depth)
                    .is_err()
            );
        }

        let after = stack(&db, target, 0).unwrap();
        assert_eq!(before, after, "rejected mutations must not touch the database");
    }

    #[test]
    fn revision_views_are_bounded_to_the_most_recent_window() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        for round in 0..(MAX_REVISIONS_IN_VIEW + 10) {
            save_section(
                &db,
                target,
                prompts::BRIDGE_ROLE_SECTION_ID,
                0,
                &format!("v{round}"),
            )
            .unwrap();
        }
        let view = stack(&db, target, 0).unwrap();
        let role = section(&view, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(role.revisions.len(), MAX_REVISIONS_IN_VIEW);
        // The window keeps the newest revisions, oldest-first within it.
        assert_eq!(
            role.revisions.first().unwrap().id,
            role.revisions.last().unwrap().id - (MAX_REVISIONS_IN_VIEW as i64 - 1)
        );

        // An older, out-of-window revision is still addressable by id.
        let oldest_kept = role.revisions.first().unwrap().id;
        let restored = restore_revision(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, oldest_kept - 1, 0);
        assert!(restored.is_ok(), "restore must reach beyond the view window");
    }

    #[test]
    fn live_and_studio_share_one_compiler_builder() {
        // The preview path and the live-turn path must produce byte-identical
        // compilers from the same resolved stack: one builder, two callers.
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Shared builder.").unwrap();
        let resolved = prompt_sections::resolve(&db, target, 0).unwrap();

        let via_shared = prompt_compiler::compiler_for_resolved_stack(&resolved)
            .unwrap()
            .compile()
            .unwrap();
        let via_live = crate::live_turn::compiler_for_stack(&resolved, target)
            .unwrap()
            .compile()
            .unwrap();
        assert_eq!(via_shared.stable_prefix, via_live.stable_prefix);
        assert_eq!(via_shared.metadata.prefix_hash, via_live.metadata.prefix_hash);
    }

    #[test]
    fn depth_bounds_are_validated() {
        let db = db();
        let worker = prompts::PromptTarget::Worker(crate::delegation::WorkerRole::Research);
        assert!(stack(&db, worker, -1).is_err());
        assert!(stack(&db, worker, delegation::DEFAULT_MAX_DEPTH + 1).is_err());
        assert!(save_section(&db, worker, prompts::WORKER_CONTRACT_SECTION_ID, -1, "x").is_err());

        let shallow = stack(&db, worker, 0).unwrap();
        let deep = stack(&db, worker, delegation::DEFAULT_MAX_DEPTH).unwrap();
        assert_ne!(shallow.depth, deep.depth);
        assert_ne!(
            section(&shallow, prompts::WORKER_CONTRACT_SECTION_ID).default_text,
            section(&deep, prompts::WORKER_CONTRACT_SECTION_ID).default_text,
            "worker contracts embed the depth-sensitive delegation protocol"
        );

        // Non-worker targets ignore depth for content but echo it verbatim.
        let orchestrator = stack(&db, prompts::PromptTarget::Orchestrator, 1).unwrap();
        assert_eq!(orchestrator.depth, 1);
        let baseline = stack(&db, prompts::PromptTarget::Orchestrator, 0).unwrap();
        for section_id in [prompts::BRIDGE_ROLE_SECTION_ID, prompts::DELEGATION_PROTOCOL_SECTION_ID] {
            assert_eq!(
                section(&orchestrator, section_id).effective_text,
                section(&baseline, section_id).effective_text,
            );
        }
    }

    #[test]
    fn preview_bytes_are_exact_for_the_resolved_stack() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        let view = preview(&db, target, 0).unwrap();

        // Independently rebuild the expected compilation from the resolved
        // stack and require byte equality on both envelopes.
        let resolved = crate::prompt_sections::resolve(&db, target, 0).unwrap();
        let mut expected = prompt_compiler::PromptCompiler::new(resolved.target.compiler_role());
        for compiled_section in &resolved.sections {
            expected = expected.stable_section(&compiled_section.id, &compiled_section.text);
        }
        let expected = expected.compile().unwrap();
        assert_eq!(view.stable_prefix, expected.stable_prefix);
        assert_eq!(view.variable_suffix, expected.variable_suffix);
        assert!(view.stable_prefix.starts_with("<bridge-stable-prompt"));
        assert!(view.stable_prefix.ends_with("</bridge-stable-prompt>"));

        // The empty-variable suffix is the exact envelope shape, not prose.
        assert_eq!(
            view.variable_suffix,
            format!(
                "<bridge-variable-context>\n{{\"sections\":[]}}\n</bridge-variable-context>"
            )
        );

        assert_eq!(view.schema_version, crate::prompt_compiler::PROMPT_SCHEMA_VERSION);
        assert_eq!(view.prefix_bytes, view.stable_prefix.len() as u64);
        assert_eq!(view.prefix_token_estimate, view.prefix_bytes.div_ceil(4));
        assert_eq!(view.prefix_hash.len(), 64);
        assert!(view.prefix_id.starts_with(&format!("bridge-prompt-v{}", view.schema_version)));
        assert_eq!(view.stack.target, "orchestrator");
    }

    #[test]
    fn preview_reflects_overrides_deletions_and_depth() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        let baseline = preview(&db, target, 0).unwrap();

        save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Custom role text.").unwrap();
        let overridden = preview(&db, target, 0).unwrap();
        assert_ne!(overridden.stable_prefix, baseline.stable_prefix);
        assert_ne!(overridden.prefix_hash, baseline.prefix_hash);
        assert!(overridden.stable_prefix.contains("Custom role text."));
        assert!(overridden.stack.sections.iter().any(|section| section.state
            == prompt_sections::PromptSectionState::Overridden {
                text: "Custom role text.".into()
            }));

        reset_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0).unwrap();
        delete_section_preview_reset(&db, target);
        let deleted = preview(&db, target, 0).unwrap();
        assert_ne!(deleted.stable_prefix, baseline.stable_prefix);
        assert!(
            !deleted.stable_prefix.contains("starter orchestrator"),
            "a deleted section contributes no bytes"
        );
        assert_ne!(deleted.prefix_hash, overridden.prefix_hash);

        let worker = prompts::PromptTarget::Worker(crate::delegation::WorkerRole::Implementation);
        let shallow = preview(&db, worker, 0).unwrap();
        let deep = preview(&db, worker, delegation::DEFAULT_MAX_DEPTH).unwrap();
        assert_ne!(shallow.stable_prefix, deep.stable_prefix);
        assert_ne!(shallow.prefix_hash, deep.prefix_hash);
    }

    fn delete_section_preview_reset(db: &Connection, target: prompts::PromptTarget) {
        prompt_sections::delete_section(
            db,
            &prompt_sections::PromptSectionKey::new(target, prompts::BRIDGE_ROLE_SECTION_ID)
                .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn agent_reset_all_flows_through_studio_views() {
        let db = db();
        let target = prompts::PromptTarget::Orchestrator;
        save_section(&db, target, prompts::BRIDGE_ROLE_SECTION_ID, 0, "Temporary").unwrap();
        agent_config::reset_all(&db).unwrap();
        let after = stack(&db, target, 0).unwrap();
        let role = section(&after, prompts::BRIDGE_ROLE_SECTION_ID);
        assert_eq!(role.state, prompt_sections::PromptSectionState::Default);
        assert_eq!(role.revisions.len(), 2, "history survives settings-wide resets");
    }

    #[test]
    fn provider_layers_cover_every_registered_adapter_with_honest_sources() {
        let db = db();
        let view = preview(&db, prompts::PromptTarget::Orchestrator, 0).unwrap();
        let authorities = prompt_authority::provider_base_prompt_authorities();
        assert_eq!(view.provider_layers.len(), authorities.len());
        for (status, authority) in view.provider_layers.iter().zip(authorities) {
            assert_eq!(status.layer, "provider_base");
            assert_eq!(status.adapter, authority.adapter);
            assert_eq!(status.bytes, None, "provider-base bytes must never be fabricated");
            match status.source {
                PromptLayerSource::Reported | PromptLayerSource::Measured => panic!(
                    "{} claims {:?} without an adapter capture behind it",
                    status.adapter, status.source
                ),
                PromptLayerSource::Estimated | PromptLayerSource::Unavailable => {}
            }
            let detail = status.detail.as_deref().unwrap_or_default();
            assert!(!detail.is_empty(), "{} needs a reason worth reading", status.adapter);
            match authority.readable {
                prompt_authority::PromptVerdict::Unsupported { reason } => {
                    assert_eq!(status.source, PromptLayerSource::Unavailable);
                    assert_eq!(detail, reason);
                }
                prompt_authority::PromptVerdict::Supported => {
                    assert!(detail.contains("does not capture"), "{detail}");
                }
            }
        }
    }
}
