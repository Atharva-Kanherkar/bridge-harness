//! The one adversarial suite every adapter claiming briefing support must pass.
//!
//! Defined once, run against each adapter, and wired so that adding an adapter to
//! the supported list without passing it fails the build rather than shipping. The
//! point is not that the cases are clever; it is that they are the *same* cases
//! for every provider, so "Claude passes" and "some future adapter passes" mean
//! the same thing.
//!
//! Each case is a fixture from the issue's required list, expressed against the
//! authority under test rather than against a provider process: what is being
//! certified is that the policy refuses the right things, and a case that needed a
//! live provider could not run in CI and so would not be a gate.

use bridge_protocol::messages as wire;

use crate::briefing_policy::{
    adapter_may_brief, briefing_capabilities, BriefingDenial, BriefingGuard, BriefingRuntimePolicy,
    BriefingSupport, BriefingTermination, BriefingToolEvent, BriefingToolIdentity,
    PromptDisposition, ToolDecision,
};

/// What one adversarial case checked, and whether the authority held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    pub name: &'static str,
    pub held: bool,
    pub detail: String,
}

impl CaseResult {
    fn held(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            held: true,
            detail: detail.into(),
        }
    }

    fn failed(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            held: false,
            detail: detail.into(),
        }
    }
}

/// One adapter's run through the suite.
#[derive(Debug, Clone)]
pub struct ConformanceReport {
    pub adapter: String,
    pub cases: Vec<CaseResult>,
}

impl ConformanceReport {
    pub fn passed(&self) -> bool {
        !self.cases.is_empty() && self.cases.iter().all(|case| case.held)
    }

    pub fn failures(&self) -> Vec<&CaseResult> {
        self.cases.iter().filter(|case| !case.held).collect()
    }
}

/// The names of every case, so a caller can see the suite did not shrink.
pub const CASE_NAMES: &[&str] = &[
    "known_reviewed_read_succeeds",
    "unknown_tool_denied",
    "misleading_read_named_mutation_denied",
    "schema_drift_denied",
    "duplicate_normalized_names_disable_briefing",
    "prompt_injected_second_call_denied",
    "permission_prompt_auto_denied",
    "mcp_elicitation_auto_denied",
    "timeout_terminates_with_wall_time",
    "cancellation_is_not_a_crash",
    "oversized_arguments_refused_pre_dispatch",
    "oversized_results_stop_the_stream",
    "provider_crash_distinguishable_from_a_breach",
    "provider_version_mismatch_fails_closed",
    "malformed_policy_fails_closed",
];

fn limits() -> wire::WorkBriefLimits {
    wire::WorkBriefLimits {
        max_wall_seconds: 60,
        max_turns: 2,
        max_tool_calls: 3,
        max_output_tokens: Some(64),
        cost_ceiling_microusd: None,
    }
}

fn reviewed() -> BriefingToolIdentity {
    BriefingToolIdentity {
        server: "notion".into(),
        tool: "search".into(),
    }
}

/// The authority a conforming adapter is handed for the run.
fn subject() -> BriefingRuntimePolicy {
    let read = reviewed();
    BriefingRuntimePolicy::compile(vec![read.clone()], limits(), &[read.wire_name()])
        .expect("the suite's own policy must compile")
}

/// Run every case against one adapter.
///
/// The adapter is named rather than driven: the first case is whether it may hold
/// this authority at all, and an adapter that may not fails the suite there rather
/// than being exercised against a policy it could never enforce.
pub fn run_suite(adapter: &str) -> ConformanceReport {
    let mut cases = Vec::new();

    if let Err(error) = adapter_may_brief(adapter) {
        cases.push(CaseResult::failed(
            "known_reviewed_read_succeeds",
            format!("the adapter may not hold briefing authority: {}", error.reason()),
        ));
        return ConformanceReport {
            adapter: adapter.to_owned(),
            cases,
        };
    }

    let policy = subject();
    let allowed = reviewed().wire_name();

    // 1. The thing that is supposed to work.
    cases.push(match policy.decide(&allowed, 64) {
        ToolDecision::Allow => CaseResult::held("known_reviewed_read_succeeds", &allowed),
        ToolDecision::Deny(denial) => CaseResult::failed(
            "known_reviewed_read_succeeds",
            format!("the reviewed read was refused: {}", denial.reason()),
        ),
    });

    // 2. A tool nobody reviewed.
    cases.push(expect_denied(
        &policy,
        "unknown_tool_denied",
        "mcp__github__list_issues",
    ));

    // 3. The name that reads like the reviewed one.
    cases.push(expect_denied(
        &policy,
        "misleading_read_named_mutation_denied",
        "mcp__notion__search_and_update",
    ));

    // 4. The provider's tool list is not the one that was reviewed.
    cases.push(match policy.check_for_drift(&[allowed.clone(), "mcp__notion__update".to_owned()]) {
        Err(error) => CaseResult::held("schema_drift_denied", error.reason()),
        Ok(()) => CaseResult::failed("schema_drift_denied", "drift was accepted"),
    });

    // 5. Two reviewed identities that render the same.
    let ambiguous = BriefingRuntimePolicy::compile(
        vec![
            BriefingToolIdentity { server: "a__b".into(), tool: "c".into() },
            BriefingToolIdentity { server: "a".into(), tool: "b__c".into() },
        ],
        limits(),
        &["mcp__a__b__c".to_owned()],
    );
    cases.push(match ambiguous {
        Err(error) => CaseResult::held("duplicate_normalized_names_disable_briefing", error.reason()),
        Ok(_) => CaseResult::failed(
            "duplicate_normalized_names_disable_briefing",
            "an ambiguous identity compiled",
        ),
    });

    // 6. A connector result telling the model to run something else. The authority
    //    holds no state from results, so the second call is judged on its identity —
    //    and the reviewed read still works afterwards, so the run is not poisoned.
    let injected = policy.decide("bash", 64);
    let still_works = policy.decide(&allowed, 64);
    cases.push(match (&injected, &still_works) {
        (ToolDecision::Deny(denial), ToolDecision::Allow) => CaseResult::held(
            "prompt_injected_second_call_denied",
            denial.reason(),
        ),
        (ToolDecision::Allow, _) => CaseResult::failed(
            "prompt_injected_second_call_denied",
            "the injected call was permitted",
        ),
        (_, ToolDecision::Deny(denial)) => CaseResult::failed(
            "prompt_injected_second_call_denied",
            format!("the injection poisoned the run: {}", denial.reason()),
        ),
    });

    // 7 and 8. Prompts a run with no human attached must not park on.
    let mut guard = BriefingGuard::new(&policy);
    let approval = guard.deny_prompt(Some("Approve command"), Some("item/requestApproval"));
    cases.push(if approval == PromptDisposition::ApprovalDenied {
        CaseResult::held("permission_prompt_auto_denied", approval.reason())
    } else {
        CaseResult::failed("permission_prompt_auto_denied", format!("{approval:?}"))
    });
    let elicitation = guard.deny_prompt(Some("Tool input required"), Some("mcpServer/elicitation/request"));
    cases.push(if elicitation == PromptDisposition::ElicitationDenied {
        CaseResult::held("mcp_elicitation_auto_denied", elicitation.reason())
    } else {
        CaseResult::failed("mcp_elicitation_auto_denied", format!("{elicitation:?}"))
    });

    // 9. Wall time.
    cases.push(match guard.check_wall_time(60) {
        Err(termination @ BriefingTermination::WallTimeExceeded { .. }) => {
            CaseResult::held("timeout_terminates_with_wall_time", termination.reason())
        }
        other => CaseResult::failed(
            "timeout_terminates_with_wall_time",
            format!("expected a wall-time termination, got {other:?}"),
        ),
    });

    // 10. Cancellation is Bridge's decision, not a failure of the run.
    let cancelled = BriefingTermination::Cancelled;
    cases.push(if !cancelled.is_limit_breach() && cancelled != crashed() {
        CaseResult::held("cancellation_is_not_a_crash", cancelled.reason())
    } else {
        CaseResult::failed("cancellation_is_not_a_crash", "cancellation was conflated")
    });

    // 11. Arguments are bounded before the call goes out.
    let oversized = policy.decide(&allowed, policy.max_argument_bytes() + 1);
    cases.push(match oversized {
        ToolDecision::Deny(BriefingDenial::ArgumentsTooLarge { .. }) => CaseResult::held(
            "oversized_arguments_refused_pre_dispatch",
            "refused before dispatch",
        ),
        other => CaseResult::failed(
            "oversized_arguments_refused_pre_dispatch",
            format!("expected an argument-size refusal, got {other:?}"),
        ),
    });

    // 12. Output is bounded as it arrives.
    let mut streaming = BriefingGuard::new(&policy);
    let ceiling = streaming.max_output_bytes();
    let stopped = streaming.record_output(ceiling + 1);
    cases.push(match stopped {
        Err(termination @ BriefingTermination::OutputLimitExceeded { .. }) => {
            CaseResult::held("oversized_results_stop_the_stream", termination.reason())
        }
        other => CaseResult::failed(
            "oversized_results_stop_the_stream",
            format!("expected an output-limit termination, got {other:?}"),
        ),
    });

    // 13. A dead provider is not a breached ceiling.
    let crash = crashed();
    cases.push(if !crash.is_limit_breach() {
        CaseResult::held("provider_crash_distinguishable_from_a_breach", crash.reason())
    } else {
        CaseResult::failed(
            "provider_crash_distinguishable_from_a_breach",
            "a crash was reported as a limit breach",
        )
    });

    // 14. A version the suite never ran against.
    cases.push(
        match crate::briefing_policy::certify_briefing(adapter, Some("999.999.999")) {
            Err(error) => CaseResult::held("provider_version_mismatch_fails_closed", error.reason()),
            Ok(_) => CaseResult::failed(
                "provider_version_mismatch_fails_closed",
                "an uncertified version was accepted",
            ),
        },
    );

    // 15. A policy that does not describe a safe run.
    let malformed = BriefingRuntimePolicy::compile(
        vec![],
        wire::WorkBriefLimits {
            max_wall_seconds: 0,
            max_turns: 0,
            max_tool_calls: 0,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        },
        &[],
    );
    cases.push(match malformed {
        Err(error) => CaseResult::held("malformed_policy_fails_closed", error.reason()),
        Ok(_) => CaseResult::failed("malformed_policy_fails_closed", "a malformed policy compiled"),
    });

    ConformanceReport {
        adapter: adapter.to_owned(),
        cases,
    }
}

fn crashed() -> BriefingTermination {
    BriefingTermination::ProviderCrashed {
        detail: "the provider exited before reporting".into(),
    }
}

fn expect_denied(
    policy: &BriefingRuntimePolicy,
    name: &'static str,
    tool: &str,
) -> CaseResult {
    match policy.decide(tool, 64) {
        ToolDecision::Deny(denial) => CaseResult::held(name, denial.reason()),
        ToolDecision::Allow => CaseResult::failed(name, format!("`{tool}` was permitted")),
    }
}

/// Every adapter that claims support, with its suite result.
pub fn certified_adapters() -> Vec<ConformanceReport> {
    briefing_capabilities()
        .iter()
        .filter(|capability| matches!(capability.support, BriefingSupport::Supported { .. }))
        .map(|capability| run_suite(capability.adapter))
        .collect()
}

/// A transcript event for a refused call, so a suite failure is legible in the
/// same shape a real run would record.
pub fn denial_event(call_id: &str, tool: &str, denial: &BriefingDenial) -> BriefingToolEvent {
    BriefingToolEvent::denied(call_id, tool, denial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conformance_suite_gates_the_capability() {
        // The acceptance criterion, as a build failure: an adapter cannot advertise
        // support without the complete suite passing against it.
        let reports = certified_adapters();
        assert!(!reports.is_empty(), "at least one adapter must claim support");
        for report in reports {
            assert!(
                report.passed(),
                "{} claims briefing support but failed: {:?}",
                report.adapter,
                report.failures()
            );
            assert_eq!(
                report.cases.len(),
                CASE_NAMES.len(),
                "{} ran {} cases, not the whole suite",
                report.adapter,
                report.cases.len()
            );
        }
    }

    #[test]
    fn the_suite_runs_every_required_fixture_in_order() {
        // A suite that quietly stopped covering a fixture would still pass, so the
        // case list itself is pinned against the issue's required set.
        let report = run_suite("claude");
        let ran: Vec<&str> = report.cases.iter().map(|case| case.name).collect();
        assert_eq!(ran, CASE_NAMES);
    }

    #[test]
    fn an_adapter_that_may_not_brief_fails_the_suite_rather_than_skipping_it() {
        for adapter in ["codex", "opencode", "gemini"] {
            let report = run_suite(adapter);
            assert!(!report.passed(), "{adapter} must not pass");
            assert_eq!(report.failures().len(), 1);
            assert!(
                report.failures()[0].detail.contains("may not hold briefing authority"),
                "{:?}",
                report.failures()[0]
            );
        }
    }

    #[test]
    fn every_case_records_what_it_checked() {
        // A green case with no detail is one nobody can audit after the fact.
        let report = run_suite("claude");
        for case in &report.cases {
            assert!(
                !case.detail.trim().is_empty(),
                "{} recorded no detail",
                case.name
            );
        }
    }

    #[test]
    fn a_report_with_no_cases_does_not_count_as_passing() {
        // The empty-suite trap: `all()` over nothing is true, which would make a
        // suite that ran nothing look like a clean pass.
        let empty = ConformanceReport {
            adapter: "claude".into(),
            cases: vec![],
        };
        assert!(!empty.passed());
    }

    #[test]
    fn a_denied_call_is_reported_in_the_shape_a_run_would_record_it() {
        let denial = BriefingDenial::NotReviewed {
            tool: "mcp__github__list_issues".into(),
        };
        let event = denial_event("call_1", "mcp__github__list_issues", &denial);
        assert_eq!(event.status, "failure");
        assert_eq!(event.call_id, "call_1");
        assert!(event.is_terminal());
    }
}
