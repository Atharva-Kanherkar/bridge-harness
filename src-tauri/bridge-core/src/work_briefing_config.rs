//! Turning Work's optional briefing configuration into something runnable.
//!
//! Briefing sits deliberately outside `ProfilePurpose`. Profile validation demands
//! exactly one profile per purpose, so a tenth purpose would make every install
//! without a briefing model *invalid* — and a feature nobody has configured must not
//! break the configuration of everything else. So it is one optional field naming a
//! harness, a model, and an effort.
//!
//! Everything here is about failing out loud. An unconfigured briefing is a state, not
//! an error. A configuration naming something Bridge cannot run is an error, and it
//! names what it could not run — never a quiet substitution, because a briefing on a
//! provider the user did not pick is a different briefing.

use bridge_protocol::messages as wire;
use serde::{Deserialize, Serialize};

use crate::briefing_policy::{certify_briefing, BriefingUnsupported};
use crate::suggestion_engine::SUGGESTION_SESSION_KIND;

/// The session kind a briefing run happens under.
///
/// Hidden everywhere a human looks. The transcript exists so a run can be inspected
/// after the fact, not so it can be joined in progress.
pub const BRIEFING_SESSION_KIND: &str = "briefing";

/// Why no briefing can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BriefingUnavailable {
    /// Nobody has chosen a briefing model. Not a failure — Work is useful without one.
    NotConfigured,
    /// A harness id that is not registered at all.
    UnknownHarness { harness: String },
    /// A registered harness that cannot hold briefing authority, or is not the version
    /// the conformance suite certified.
    ProviderUnsupported { harness: String, reason: String },
    /// Configuration that does not describe a run.
    Malformed { detail: String },
}

impl BriefingUnavailable {
    /// The stable code recorded on a skipped run.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::UnknownHarness { .. } => "provider_unknown",
            Self::ProviderUnsupported { .. } => "provider_unsupported",
            Self::Malformed { .. } => "configuration_invalid",
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::NotConfigured => "no briefing model is configured, so Work is facts-only".into(),
            Self::UnknownHarness { harness } => {
                format!("`{harness}` is not a harness Bridge knows about")
            }
            Self::ProviderUnsupported { harness, reason } => {
                format!("`{harness}` cannot run a briefing: {reason}")
            }
            Self::Malformed { detail } => format!("the briefing configuration is not usable: {detail}"),
        }
    }

    /// What the board shows for suggestions given this.
    pub fn suggestions_state(&self) -> wire::WorkSuggestionsState {
        match self {
            Self::NotConfigured => wire::WorkSuggestionsState::NotConfigured,
            _ => wire::WorkSuggestionsState::ProviderUnsupported,
        }
    }
}

/// A briefing that can actually be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingSelection {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    /// The version the conformance suite certified for this harness, carried so the run
    /// row records what it was checked against rather than what it happened to meet.
    pub certified_provider_version: String,
}

impl BriefingSelection {
    /// A stable string identifying this selection, for the run's `profile_reference`.
    /// Provenance, not a key: it says which configuration produced a run after that
    /// configuration has moved on.
    pub fn reference(&self) -> String {
        match &self.effort {
            Some(effort) => format!("{}/{}/{}", self.harness, self.model, effort),
            None => format!("{}/{}", self.harness, self.model),
        }
    }
}

/// Resolve Work's settings into a runnable selection.
///
/// `reported_versions` is what each registered harness says it is right now, so the
/// certification check is against reality rather than against a table. A harness absent
/// from it is unknown — not assumed present, because assuming would be the one path
/// here that could run something uncertified.
pub fn resolve_briefing(
    settings: &wire::WorkSettings,
    reported_versions: &dyn Fn(&str) -> Option<String>,
) -> Result<BriefingSelection, BriefingUnavailable> {
    let Some(profile) = &settings.briefing else {
        return Err(BriefingUnavailable::NotConfigured);
    };
    // HarnessId is an open validated newtype, not a closed enum: a third-party ACP
    // harness can be configured, resolves by name, and is refused below as unknown.
    // That is the fail-closed reading, and it needs no list here to stay correct.
    let harness = profile.harness.as_str();
    let model = profile.model.trim();
    if model.is_empty() {
        return Err(BriefingUnavailable::Malformed {
            detail: "the briefing model is empty".into(),
        });
    }
    let Some(version) = reported_versions(harness) else {
        return Err(BriefingUnavailable::UnknownHarness {
            harness: harness.to_owned(),
        });
    };

    // The same gate slice 2 built. Reusing it means the answer to "may this provider
    // hold briefing authority" is given in exactly one place.
    let capability = certify_briefing(harness, Some(&version)).map_err(|error| match &error {
        BriefingUnsupported::UnknownAdapter { adapter } => BriefingUnavailable::UnknownHarness {
            harness: adapter.clone(),
        },
        _ => BriefingUnavailable::ProviderUnsupported {
            harness: harness.to_owned(),
            reason: error.reason(),
        },
    })?;

    let certified = match capability.support {
        crate::briefing_policy::BriefingSupport::Supported {
            certified_provider_version,
        } => certified_provider_version.to_owned(),
        // certify_briefing refuses every unsupported adapter, so this is unreachable;
        // treating it as unsupported rather than unwrapping keeps a future table edit
        // from turning a bug into a run.
        crate::briefing_policy::BriefingSupport::Unsupported { reason } => {
            return Err(BriefingUnavailable::ProviderUnsupported {
                harness: harness.to_owned(),
                reason: reason.to_owned(),
            })
        }
    };

    Ok(BriefingSelection {
        harness: harness.to_owned(),
        model: model.to_owned(),
        effort: profile.effort.map(|effort| effort_str(effort).to_owned()),
        certified_provider_version: certified,
    })
}

fn effort_str(effort: wire::Effort) -> &'static str {
    match effort {
        wire::Effort::Low => "low",
        wire::Effort::Medium => "medium",
        wire::Effort::High => "high",
        wire::Effort::Xhigh => "xhigh",
    }
}

/// Is this session one a human should ever see in a list?
///
/// A predicate rather than an ordering rule: a briefing or suggestion session that
/// merely sorted last would still be one keystroke from being opened, resumed, or
/// sent a turn.
pub fn is_hidden_session_kind(kind: Option<&str>) -> bool {
    matches!(
        kind,
        Some(BRIEFING_SESSION_KIND) | Some(SUGGESTION_SESSION_KIND)
    ) || kind == Some(crate::memory_extraction::EXTRACTION_SESSION_KIND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(briefing: Option<wire::WorkBriefingProfile>) -> wire::WorkSettings {
        wire::WorkSettings {
            briefing,
            enabled_connector_instances: vec![],
            refresh_on_focus: false,
            refresh_interval_minutes: None,
            cooldown_minutes: 15,
            limits: wire::WorkBriefLimits {
                max_wall_seconds: 600,
                max_turns: 12,
                max_tool_calls: 24,
                max_output_tokens: None,
                cost_ceiling_microusd: None,
            },
        }
    }

    fn profile(harness: &str, model: &str) -> wire::WorkBriefingProfile {
        wire::WorkBriefingProfile {
            harness: wire::HarnessId::parse(harness).unwrap(),
            model: model.into(),
            effort: Some(wire::Effort::Medium),
        }
    }

    /// Claude reports the version the conformance suite certified; nothing else is
    /// registered.
    fn registered(harness: &str) -> Option<String> {
        match harness {
            "claude" => Some("0.3.209".into()),
            "codex" => Some("1.0.0".into()),
            "opencode" => Some("1.0.0".into()),
            _ => None,
        }
    }

    #[test]
    fn briefing_configuration_is_optional_and_adds_no_profile_purpose() {
        // Nine purposes, and briefing is not one of them: a tenth would make every
        // install without a briefing model fail profile validation.
        assert_eq!(crate::model_profiles::ProfilePurpose::ALL.len(), 9);
        let unconfigured = settings(None);
        assert!(unconfigured.briefing.is_none());
    }

    #[test]
    fn an_unconfigured_briefing_is_not_configured_not_an_error() {
        let error = resolve_briefing(&settings(None), &registered).unwrap_err();
        assert_eq!(error, BriefingUnavailable::NotConfigured);
        assert_eq!(error.code(), "not_configured");
        assert_eq!(error.suggestions_state(), wire::WorkSuggestionsState::NotConfigured);
        assert!(error.reason().contains("facts-only"));
    }

    #[test]
    fn a_certified_provider_resolves() {
        let selection =
            resolve_briefing(&settings(Some(profile("claude", "sonnet"))), &registered).unwrap();
        assert_eq!(selection.harness, "claude");
        assert_eq!(selection.model, "sonnet");
        assert_eq!(selection.effort.as_deref(), Some("medium"));
        assert_eq!(selection.certified_provider_version, "0.3");
        assert_eq!(selection.reference(), "claude/sonnet/medium");
    }

    #[test]
    fn a_selection_without_an_effort_still_has_a_reference() {
        let mut chosen = profile("claude", "sonnet");
        chosen.effort = None;
        let selection = resolve_briefing(&settings(Some(chosen)), &registered).unwrap();
        assert_eq!(selection.reference(), "claude/sonnet");
    }

    #[test]
    fn an_unregistered_harness_is_unknown_rather_than_assumed_present() {
        // Assuming presence is the one path here that could run something uncertified.
        let error =
            resolve_briefing(&settings(Some(profile("gemini", "flash"))), &registered).unwrap_err();
        assert_eq!(
            error,
            BriefingUnavailable::UnknownHarness { harness: "gemini".into() }
        );
        assert_eq!(error.code(), "provider_unknown");
    }

    #[test]
    fn an_uncertified_provider_fails_with_a_code_and_no_fallback() {
        for harness in ["codex", "opencode"] {
            let error =
                resolve_briefing(&settings(Some(profile(harness, "any"))), &registered).unwrap_err();
            let BriefingUnavailable::ProviderUnsupported { harness: named, reason } = &error else {
                panic!("expected provider_unsupported for {harness}, got {error:?}");
            };
            assert_eq!(named, harness);
            assert!(!reason.is_empty());
            assert_eq!(error.code(), "provider_unsupported");
            // The refusal names the provider that was asked for, and nothing else — a
            // briefing on a provider the user did not choose is a different briefing.
            assert!(!error.reason().contains("claude"), "no substitution may be offered");
        }
    }

    #[test]
    fn a_provider_reporting_an_uncertified_version_is_refused() {
        // Certification is against what the provider says it is now, not against a
        // table entry that was true once.
        let stale = |harness: &str| match harness {
            "claude" => Some("0.2.1".into()),
            _ => None,
        };
        let error =
            resolve_briefing(&settings(Some(profile("claude", "sonnet"))), &stale).unwrap_err();
        assert_eq!(error.code(), "provider_unsupported");
        assert!(error.reason().contains("0.2.1"));
    }

    #[test]
    fn an_empty_model_is_malformed_rather_than_a_default() {
        for model in ["", "   "] {
            let error = resolve_briefing(&settings(Some(profile("claude", model))), &registered)
                .unwrap_err();
            assert!(matches!(error, BriefingUnavailable::Malformed { .. }), "{error:?}");
            assert_eq!(error.code(), "configuration_invalid");
        }
    }

    #[test]
    fn every_unavailability_maps_to_a_suggestions_state_and_a_stable_code() {
        let all = [
            BriefingUnavailable::NotConfigured,
            BriefingUnavailable::UnknownHarness { harness: "x".into() },
            BriefingUnavailable::ProviderUnsupported { harness: "x".into(), reason: "r".into() },
            BriefingUnavailable::Malformed { detail: "d".into() },
        ];
        for unavailable in &all {
            assert!(unavailable.code().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(!unavailable.reason().is_empty());
        }
        // Only the unconfigured case is the quiet one; everything else says the provider
        // cannot do it, which is a different thing for a reader to see.
        assert_eq!(all[0].suggestions_state(), wire::WorkSuggestionsState::NotConfigured);
        for unavailable in &all[1..] {
            assert_eq!(
                unavailable.suggestions_state(),
                wire::WorkSuggestionsState::ProviderUnsupported
            );
        }
    }

    #[test]
    fn hidden_session_kinds_cover_briefing_and_suggestion_and_nothing_else() {
        assert!(is_hidden_session_kind(Some(BRIEFING_SESSION_KIND)));
        assert!(is_hidden_session_kind(Some(SUGGESTION_SESSION_KIND)));
        for visible in [None, Some("orchestrator"), Some("chat"), Some("worker"), Some("")] {
            assert!(!is_hidden_session_kind(visible), "{visible:?} is a session a human may see");
        }
    }

    #[test]
    fn the_hidden_kinds_are_the_strings_shared_with_the_frontend() {
        // The frontend filters on the same literals. Keeping them constants here is what
        // makes the two halves the same rule rather than two rules that agree today.
        assert_eq!(BRIEFING_SESSION_KIND, "briefing");
        assert_eq!(SUGGESTION_SESSION_KIND, "suggestion");
    }
}
