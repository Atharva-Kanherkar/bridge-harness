//! One briefing run, start to finish.
//!
//! Everything the run needs already exists: slice 2 decides what a tool call may do,
//! and steps 1–5 supply the parser, the connector registry, the ledger, the store, and
//! the configuration. This is the order they happen in, and the places a run stops.
//!
//! The runner is written against a trait rather than a provider process, for a reason
//! that is about testing but also about honesty: a briefing is a sequence of decisions,
//! and the decisions are what this slice is responsible for. Whether Claude's sidecar
//! returns bytes is slice 2's conformance suite and the live app; whether a run refuses
//! a fabricated citation is here, and it should be checkable without a network.

use bridge_protocol::messages as wire;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::briefing_policy::{BriefingGuard, BriefingRuntimePolicy, BriefingToolIdentity, ToolDecision};
use crate::work_brief_parser::{parse_with_one_repair, BriefOutcome, BriefRejection, WorkBrief};
use crate::work_brief_store::{begin_run, RunOutcome, RunStart};
use crate::work_briefing_config::{BriefingSelection, BriefingUnavailable};
use crate::work_connectors::{eligibility, ConnectorEligibility, ConnectorInstance};
use crate::work_evidence::RunLedger;
use crate::work_reconcile::{self, Commit};
use crate::BridgeError;

/// One tool call a provider asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub call_id: String,
    /// The wire name, as the provider used it.
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// What a connector returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResult {
    Succeeded(serde_json::Value),
    Failed { detail: String },
}

/// The provider side of a run, reduced to what a briefing actually needs from it.
pub trait BriefingProvider {
    /// Start the run, told which tools it may use.
    fn open(&mut self, offered: &[BriefingToolIdentity]) -> Result<(), String>;
    /// The calls the provider wants to make before it answers. Called until empty.
    fn pending_calls(&mut self) -> Vec<ToolCall>;
    /// Hand a call's outcome back.
    ///
    /// `evidence_ref` is the reference this call earned, and `None` when it earned
    /// nothing. It is the only way a provider can know what to cite: a brief names
    /// evidence by reference, and a model that had to guess the reference format would
    /// be authoring provenance again by the back door.
    fn deliver(&mut self, call_id: &str, outcome: &ToolResult, evidence_ref: Option<&str>);
    /// Ask for the brief, once the calls are done.
    fn answer(&mut self) -> Result<String, String>;
    /// Ask for a repair, given why the last answer was refused. `None` if the provider
    /// cannot be asked again.
    fn repair(&mut self, rejection: &BriefRejection) -> Option<String>;
    fn usage(&self) -> Option<wire::WorkRunUsage>;
}

/// What connectors a run may read, and how to read them.
pub trait ConnectorSource {
    fn instances(&self) -> Vec<ConnectorInstance>;
    /// Perform one allowed call. Only reached for a call the policy permitted.
    fn call(&mut self, call: &ToolCall) -> ToolResult;
}

/// How a run ended, for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResult {
    /// A brief was accepted and committed to the durable board.
    Accepted { run_id: String, brief: WorkBrief, repaired: bool },
    /// The run happened and produced nothing usable.
    Failed { run_id: String, code: String },
    /// The run never started.
    Skipped { code: String },
}

/// Digest an accepted brief, so a run records what it produced without keeping a second
/// unbounded copy of the model's prose.
fn output_digest(brief: &WorkBrief) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"bridge-work-brief-v1\0");
    hasher.update(serde_json::to_string(brief).unwrap_or_default().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Everything one run needs that is not a trait.
pub struct RunRequest<'a> {
    pub run_id: &'a str,
    pub trigger: wire::WorkBriefTrigger,
    pub selection: &'a BriefingSelection,
    pub limits: wire::WorkBriefLimits,
    pub session_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
    pub started_at: &'a str,
    pub now: &'a dyn Fn() -> String,
}

/// Run a briefing.
///
/// The shape to notice: every exit writes the run row before returning. A run that
/// stopped is a run somebody can ask about, and a run that vanished is one they cannot.
pub fn run_briefing(
    db: &mut Connection,
    request: RunRequest<'_>,
    connectors: &mut dyn ConnectorSource,
    provider: &mut dyn BriefingProvider,
) -> Result<RunResult, BridgeError> {
    let mut ledger = RunLedger::new(request.run_id);

    // Which connectors may be offered. Recorded first, so a run that finds nothing to
    // read still explains why rather than looking like it did not try.
    let mut offered: Vec<BriefingToolIdentity> = Vec::new();
    let instances = connectors.instances();
    for instance in &instances {
        match eligibility(instance) {
            ConnectorEligibility::Eligible { family, tools } => {
                ledger.record_source(
                    &instance.instance_id,
                    family.as_str(),
                    wire::WorkSourceStatus::Eligible,
                    None,
                    None,
                );
                offered.extend(tools);
            }
            ConnectorEligibility::Ineligible(reason) => {
                let status = match reason {
                    crate::work_connectors::IneligibleReason::AuthRequired => {
                        wire::WorkSourceStatus::AuthRequired
                    }
                    _ => wire::WorkSourceStatus::Ineligible,
                };
                ledger.record_source(
                    &instance.instance_id,
                    instance.family.map(|family| family.as_str()).unwrap_or("unknown"),
                    status,
                    Some(reason.reason()),
                    None,
                );
            }
        }
    }

    begin_run(
        db,
        &RunStart {
            run_id: request.run_id.to_owned(),
            trigger: request.trigger,
            profile_reference: Some(request.selection.reference()),
            session_id: request.session_id.map(str::to_owned),
            limits: request.limits.clone(),
            idempotency_key: request.idempotency_key.map(str::to_owned),
            started_at: request.started_at.to_owned(),
            // The proxied runner is claimed by its caller; the lease, when one
            // exists, was written by the claim path before this ran.
            lease_owner: None,
            lease_expires_at: None,
        },
    )?;

    // The authority for this run. Compiled against exactly what was offered, so drift
    // between now and the call is caught by the policy rather than by nobody.
    let presented: Vec<String> = offered.iter().map(BriefingToolIdentity::wire_name).collect();
    let policy = match BriefingRuntimePolicy::compile(offered.clone(), request.limits.clone(), &presented) {
        Ok(policy) => policy,
        Err(unsupported) => {
            return finish_failed(db, &mut ledger, &request, "policy_invalid", Some(unsupported.reason()), provider)
        }
    };
    let mut guard = BriefingGuard::new(&policy);

    if let Err(detail) = provider.open(&offered) {
        return finish_failed(db, &mut ledger, &request, "provider_failed", Some(detail), provider);
    }

    // Serve the provider's calls. Each one is decided by the policy, and only a call it
    // allowed reaches a connector at all.
    let mut turns = 0i64;
    loop {
        let calls = provider.pending_calls();
        if calls.is_empty() {
            break;
        }
        if guard.begin_turn().is_err() {
            return finish_failed(db, &mut ledger, &request, "budget_exceeded", Some("turn limit".into()), provider);
        }
        turns += 1;
        for call in calls {
            if guard.begin_tool_call().is_err() {
                return finish_failed(db, &mut ledger, &request, "budget_exceeded", Some("tool-call limit".into()), provider);
            }
            let bytes = serde_json::to_string(&call.arguments).map(|value| value.len()).unwrap_or(usize::MAX);
            let (instance, family) = match locate(&instances, &call.tool) {
                Some(found) => found,
                // A call naming no known connector cannot be served and earns nothing;
                // the policy would refuse it anyway, and this keeps the coverage row
                // from being attributed to a connector that was never involved.
                None => {
                    provider.deliver(
                        &call.call_id,
                        &ToolResult::Failed { detail: "unknown tool".into() },
                        None,
                    );
                    continue;
                }
            };
            if let ToolDecision::Deny(denial) = policy.decide(&call.tool, bytes) {
                // Denied: the model tried, which is worth recording, but nothing is
                // earned and no connector is touched.
                ledger.record_consulted(&instance.instance_id, family.as_str());
                provider.deliver(
                    &call.call_id,
                    &ToolResult::Failed { detail: denial.reason() },
                    None,
                );
                continue;
            }
            ledger.record_consulted(&instance.instance_id, family.as_str());
            let outcome = connectors.call(&call);
            // The reference this call earned, handed straight back so the provider can
            // cite it. Nothing else in the run tells it what exists.
            let earned = match &outcome {
                ToolResult::Succeeded(result) => {
                    let digest = instance
                        .reviewed_tools
                        .iter()
                        .find(|reviewed| reviewed.identity.wire_name() == call.tool)
                        .map(|reviewed| reviewed.definition_digest.clone())
                        .unwrap_or_default();
                    ledger.record_succeeded(
                        family,
                        &instance.instance_id,
                        instance.account_identity.as_deref(),
                        &call.call_id,
                        &digest,
                        result,
                        &(request.now)(),
                    )
                }
                ToolResult::Failed { detail } => {
                    ledger.record_failed(&instance.instance_id, family.as_str(), detail.clone());
                    None
                }
            };
            provider.deliver(&call.call_id, &outcome, earned.as_deref());
        }
    }

    // Now ask for the brief. After the calls, so the provider has been told every
    // reference it earned and a citation is something it was given rather than guessed.
    let answer = match provider.answer() {
        Ok(message) => message,
        Err(detail) => {
            return finish_failed(db, &mut ledger, &request, "provider_failed", Some(detail), provider)
        }
    };
    let outcome = parse_with_one_repair(&answer, &ledger, |rejection| provider.repair(rejection));
    match outcome {
        BriefOutcome::Accepted { brief, repaired } => {
            let completed_at = (request.now)();
            let run_outcome = RunOutcome {
                status: wire::WorkBriefRunStatus::Succeeded,
                output_digest: Some(output_digest(&brief)),
                failure_code: None,
                failure_detail: None,
                usage: provider.usage(),
                tool_calls: guard.tool_calls(),
                turns,
                completed_at: completed_at.clone(),
            };
            work_reconcile::commit_brief(
                db,
                Commit {
                    run_id: request.run_id,
                    brief: &brief,
                    ledger: &ledger,
                    outcome: run_outcome,
                    now: &completed_at,
                },
            )?;
            Ok(RunResult::Accepted {
                run_id: request.run_id.to_owned(),
                brief,
                repaired,
            })
        }
        refused => {
            let code = refused.failure_code().unwrap_or("schema_invalid").to_owned();
            let detail = match &refused {
                BriefOutcome::Refused { first, repair } => {
                    Some(repair.as_ref().unwrap_or(first).detail())
                }
                BriefOutcome::Accepted { .. } => None,
            };
            work_reconcile::abandon_run(
                db,
                request.run_id,
                &ledger,
                &RunOutcome {
                    status: wire::WorkBriefRunStatus::Failed,
                    // No digest: nothing was accepted, and a digest of a refused payload
                    // would read as though something had been.
                    output_digest: None,
                    failure_code: Some(code.clone()),
                    failure_detail: detail,
                    usage: provider.usage(),
                    tool_calls: guard.tool_calls(),
                    turns,
                    completed_at: (request.now)(),
                },
            )?;
            Ok(RunResult::Failed {
                run_id: request.run_id.to_owned(),
                code,
            })
        }
    }
}

/// Which connector a wire name belongs to.
fn locate<'a>(
    instances: &'a [ConnectorInstance],
    tool: &str,
) -> Option<(&'a ConnectorInstance, crate::work_connectors::ConnectorFamily)> {
    instances.iter().find_map(|instance| {
        let family = instance.family?;
        instance
            .reviewed_tools
            .iter()
            .any(|reviewed| reviewed.identity.wire_name() == tool)
            .then_some((instance, family))
    })
}

/// Close a run that stopped before it could produce a brief.
///
/// The ledger is written even here: what was offered, consulted, and read is the useful
/// part of a failed run, and discarding it would leave a code with nothing behind it.
fn finish_failed(
    db: &mut Connection,
    ledger: &mut RunLedger,
    request: &RunRequest<'_>,
    code: &str,
    detail: Option<String>,
    provider: &dyn BriefingProvider,
) -> Result<RunResult, BridgeError> {
    work_reconcile::abandon_run(
        db,
        request.run_id,
        ledger,
        &RunOutcome {
            status: wire::WorkBriefRunStatus::Failed,
            output_digest: None,
            failure_code: Some(code.to_owned()),
            failure_detail: detail,
            usage: provider.usage(),
            tool_calls: 0,
            turns: 0,
            completed_at: (request.now)(),
        },
    )?;
    Ok(RunResult::Failed {
        run_id: request.run_id.to_owned(),
        code: code.to_owned(),
    })
}

/// A run that never started, because there was nothing to start it with.
pub fn skip(unavailable: &BriefingUnavailable) -> RunResult {
    RunResult::Skipped {
        code: unavailable.code().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use crate::work_brief_store::{read_coverage, read_evidence, read_run};
    use crate::work_connectors::{tool_definition_digest, ConnectorFamily, ReviewedTool};
    use serde_json::json;
    use std::collections::BTreeMap;

    const STARTED: &str = "2026-08-19T12:00:00+00:00";

    fn db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn limits() -> wire::WorkBriefLimits {
        wire::WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 4,
            max_tool_calls: 6,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        }
    }

    fn selection() -> BriefingSelection {
        BriefingSelection {
            harness: "claude".into(),
            model: "sonnet".into(),
            effort: Some("medium".into()),
            certified_provider_version: "0.3".into(),
        }
    }

    fn identity(server: &str, tool: &str) -> BriefingToolIdentity {
        BriefingToolIdentity { server: server.into(), tool: tool.into() }
    }

    const SLACK_TOOL: &str = "mcp__slack-1__search_messages";

    /// A signed-in Slack instance with one reviewed tool, a Gmail instance nobody is
    /// signed into, and a Notion instance with nothing reviewed.
    fn instances() -> Vec<ConnectorInstance> {
        let reviewed = identity("slack-1", "search_messages");
        let digest = tool_definition_digest(&json!({"name": "search"}));
        vec![
            ConnectorInstance {
                instance_id: "slack-1".into(),
                family: Some(ConnectorFamily::Slack),
                account_identity: Some("T1/U1".into()),
                authenticated: true,
                reviewed_tools: vec![ReviewedTool { identity: reviewed.clone(), definition_digest: digest.clone() }],
                presented_digests: BTreeMap::from([(reviewed.wire_name(), digest)]),
            },
            ConnectorInstance {
                instance_id: "gmail-1".into(),
                family: Some(ConnectorFamily::Gmail),
                account_identity: None,
                authenticated: false,
                reviewed_tools: vec![],
                presented_digests: BTreeMap::new(),
            },
            ConnectorInstance {
                instance_id: "notion-1".into(),
                family: Some(ConnectorFamily::Notion),
                account_identity: None,
                authenticated: true,
                reviewed_tools: vec![],
                presented_digests: BTreeMap::new(),
            },
        ]
    }

    struct Connectors {
        instances: Vec<ConnectorInstance>,
        results: Vec<ToolResult>,
        calls: Vec<String>,
    }

    impl Connectors {
        fn with(results: Vec<ToolResult>) -> Self {
            Self { instances: instances(), results, calls: vec![] }
        }
    }

    impl ConnectorSource for Connectors {
        fn instances(&self) -> Vec<ConnectorInstance> {
            self.instances.clone()
        }
        fn call(&mut self, call: &ToolCall) -> ToolResult {
            self.calls.push(call.tool.clone());
            if self.results.is_empty() {
                return ToolResult::Failed { detail: "no result staged".into() };
            }
            self.results.remove(0)
        }
    }

    /// A provider that makes the calls it was given, then cites exactly the references
    /// it was handed. Nothing here knows the reference format, which is the point: a
    /// stub that predicted `{run}:ev-N` would prove the run works only for a stub.
    struct Citing {
        rounds: Vec<Vec<ToolCall>>,
        earned: Vec<String>,
        delivered: Vec<(String, bool)>,
        repairs: usize,
        /// When set, the first answer is this instead, so a refusal can be exercised.
        bad_answer: Option<String>,
        repair_answer: Option<String>,
    }

    impl Citing {
        fn new(rounds: Vec<Vec<ToolCall>>) -> Self {
            Self { rounds, earned: vec![], delivered: vec![], repairs: 0, bad_answer: None, repair_answer: None }
        }
    }

    fn brief_citing(refs: &[String]) -> String {
        let cited: Vec<String> = refs.iter().map(|value| format!("\"{value}\"")).collect();
        format!(
            "```bridge-work-brief\n{{\"version\":1,\"tasks\":[{{\"rank\":1,\"title\":\"Reply to Priya\",\"why\":\"Asked twice.\",\"confidenceBps\":8200,\"evidence\":[{}]}}]}}\n```",
            cited.join(",")
        )
    }

    const EMPTY_BRIEF: &str = "```bridge-work-brief\n{\"version\":1,\"tasks\":[]}\n```";

    impl BriefingProvider for Citing {
        fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<(), String> {
            Ok(())
        }
        fn pending_calls(&mut self) -> Vec<ToolCall> {
            if self.rounds.is_empty() { vec![] } else { self.rounds.remove(0) }
        }
        fn deliver(&mut self, call_id: &str, outcome: &ToolResult, evidence_ref: Option<&str>) {
            self.delivered.push((call_id.to_owned(), matches!(outcome, ToolResult::Succeeded(_))));
            if let Some(reference) = evidence_ref {
                self.earned.push(reference.to_owned());
            }
        }
        fn answer(&mut self) -> Result<String, String> {
            if let Some(bad) = self.bad_answer.take() {
                return Ok(bad);
            }
            Ok(if self.earned.is_empty() { EMPTY_BRIEF.to_owned() } else { brief_citing(&self.earned) })
        }
        fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> {
            self.repairs += 1;
            self.repair_answer.clone()
        }
        fn usage(&self) -> Option<wire::WorkRunUsage> {
            None
        }
    }

    fn call(id: &str, tool: &str) -> ToolCall {
        ToolCall { call_id: id.into(), tool: tool.into(), arguments: json!({"query": "flag"}) }
    }

    fn slack_result() -> serde_json::Value {
        json!({"ts": "1723459200.123", "permalink": "https://app.slack.com/archives/C1/p1", "text": "secret sk-ant-x"})
    }

    fn request<'a>(run_id: &'a str, chosen: &'a BriefingSelection, now: &'a dyn Fn() -> String) -> RunRequest<'a> {
        RunRequest {
            run_id,
            trigger: wire::WorkBriefTrigger::Manual,
            selection: chosen,
            limits: limits(),
            session_id: None,
            idempotency_key: None,
            started_at: STARTED,
            now,
        }
    }

    #[test]
    fn a_whole_run_offers_reads_and_parses_a_brief_citing_what_it_earned() {
        // The provider is told its references rather than predicting them, which is what
        // makes this a test of the run instead of a test of the id format.
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![ToolResult::Succeeded(slack_result())]);
        let mut provider = Citing::new(vec![vec![call("c1", SLACK_TOOL)]]);

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        let RunResult::Accepted { brief, repaired, .. } = &result else {
            panic!("expected an accepted run, got {result:?}");
        };
        assert!(!repaired);
        assert_eq!(brief.tasks.len(), 1);
        // The citation is the reference the runner handed over.
        assert_eq!(provider.earned.len(), 1);
        assert_eq!(brief.tasks[0].evidence, provider.earned);

        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Succeeded);
        assert!(run.output_digest.is_some());
        assert_eq!(run.profile_reference.as_deref(), Some("claude/sonnet/medium"));

        let coverage = read_coverage(&db, "run-1").unwrap();
        let status = |id: &str| coverage.iter().find(|row| row.connector_instance_id == id).unwrap().status;
        assert_eq!(status("slack-1"), wire::WorkSourceStatus::Succeeded);
        assert_eq!(status("gmail-1"), wire::WorkSourceStatus::AuthRequired, "not a failure");
        assert_eq!(status("notion-1"), wire::WorkSourceStatus::Ineligible);

        let evidence = read_evidence(&db, "run-1").unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].canonical_resource_id.as_deref(), Some("slack:slack-1:1723459200.123"));
        assert_eq!(evidence[0].evidence_ref, provider.earned[0]);

        // An accepted run publishes its evidence and reconciled task atomically.
        let task: (i64, String, String) = db
            .query_row("SELECT COUNT(*),title,last_run_id FROM work_tasks", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(task, (1, "Reply to Priya".into(), "run-1".into()));
        let facts: i64 = db.query_row("SELECT COUNT(*) FROM work_fact_cache", [], |row| row.get(0)).unwrap();
        assert_eq!(facts, 0);
    }

    #[test]
    fn a_failed_call_is_delivered_with_no_reference() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![ToolResult::Failed { detail: "503 from Slack".into() }]);
        let mut provider = Citing::new(vec![vec![call("c1", SLACK_TOOL)]]);

        run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        assert_eq!(connectors.calls.len(), 1, "the allowed call did reach the connector");
        assert!(provider.earned.is_empty(), "a failure vouches for nothing, so it cites nothing");
        assert_eq!(provider.delivered, vec![("c1".to_owned(), false)]);
        assert!(read_evidence(&db, "run-1").unwrap().is_empty());
        let coverage = read_coverage(&db, "run-1").unwrap();
        let slack = coverage.iter().find(|row| row.connector_instance_id == "slack-1").unwrap();
        assert_eq!(slack.status, wire::WorkSourceStatus::Failed);
        assert_eq!(slack.detail.as_deref(), Some("503 from Slack"));
    }

    #[test]
    fn a_policy_denied_call_is_recorded_as_consulted_and_touches_no_connector() {
        // A real policy deny, not the unknown-tool branch: the tool is reviewed and
        // offered, and the arguments are over the ceiling. Found in review — the earlier
        // version of this test asked for an unreviewed tool and so never reached the
        // deny arm at all.
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![ToolResult::Succeeded(slack_result())]);
        let oversized = ToolCall {
            call_id: "c1".into(),
            tool: SLACK_TOOL.into(),
            arguments: json!({"query": "x".repeat(16 * 1024)}),
        };
        let mut provider = Citing::new(vec![vec![oversized]]);

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        assert!(matches!(result, RunResult::Accepted { .. }), "an empty brief is a valid answer");
        assert!(connectors.calls.is_empty(), "a denied call never reaches the connector");
        assert!(provider.earned.is_empty(), "and earns nothing to cite");
        assert_eq!(provider.delivered, vec![("c1".to_owned(), false)]);
        // The model tried, which is worth recording.
        let coverage = read_coverage(&db, "run-1").unwrap();
        let slack = coverage.iter().find(|row| row.connector_instance_id == "slack-1").unwrap();
        assert_eq!(slack.status, wire::WorkSourceStatus::Consulted);
        assert!(read_evidence(&db, "run-1").unwrap().is_empty());
    }

    #[test]
    fn a_call_naming_no_known_connector_is_not_attributed_to_one() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![]);
        let mut provider = Citing::new(vec![vec![call("c1", "mcp__slack-1__post_message")]]);

        run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        assert!(connectors.calls.is_empty());
        let coverage = read_coverage(&db, "run-1").unwrap();
        let slack = coverage.iter().find(|row| row.connector_instance_id == "slack-1").unwrap();
        assert_eq!(
            slack.status,
            wire::WorkSourceStatus::Eligible,
            "an unreviewed tool is nobody's call, so no connector is marked consulted"
        );
    }

    #[test]
    fn a_fabricated_citation_fails_the_run_after_exactly_one_repair() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![ToolResult::Succeeded(slack_result())]);
        let mut provider = Citing::new(vec![vec![call("c1", SLACK_TOOL)]]);
        let fabricated = brief_citing(&["run-1:ev-99".to_owned()]);
        provider.bad_answer = Some(fabricated.clone());
        provider.repair_answer = Some(brief_citing(&["run-1:ev-98".to_owned()]));

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        assert_eq!(result, RunResult::Failed { run_id: "run-1".into(), code: "evidence_invalid".into() });
        assert_eq!(provider.repairs, 1, "exactly one repair, however many would fail");

        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.failure_code.as_deref(), Some("evidence_invalid"));
        assert!(run.output_digest.is_none(), "nothing was accepted, so nothing is digested");
        // The useful part of a failed run survives: the evidence it did earn, and why
        // each source got where it did.
        assert_eq!(read_evidence(&db, "run-1").unwrap().len(), 1);
        assert_eq!(read_coverage(&db, "run-1").unwrap().len(), 3);
    }

    #[test]
    fn a_provider_that_never_opens_still_leaves_a_run_to_ask_about() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![]);
        struct Broken;
        impl BriefingProvider for Broken {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<(), String> {
                Err("the sidecar exited before answering".into())
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> { vec![] }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult, _earned: Option<&str>) {}
            fn answer(&mut self) -> Result<String, String> { Ok(String::new()) }
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Broken).unwrap();
        assert_eq!(result, RunResult::Failed { run_id: "run-1".into(), code: "provider_failed".into() });
        assert_eq!(read_coverage(&db, "run-1").unwrap().len(), 3);
    }

    #[test]
    fn a_provider_that_dies_before_answering_is_a_failed_run_not_a_lost_one() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![ToolResult::Succeeded(slack_result())]);
        struct Mute(bool);
        impl BriefingProvider for Mute {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<(), String> { Ok(()) }
            fn pending_calls(&mut self) -> Vec<ToolCall> {
                if self.0 { return vec![] }
                self.0 = true;
                vec![call("c1", SLACK_TOOL)]
            }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult, _earned: Option<&str>) {}
            fn answer(&mut self) -> Result<String, String> { Err("the sidecar stopped".into()) }
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Mute(false)).unwrap();
        assert_eq!(result, RunResult::Failed { run_id: "run-1".into(), code: "provider_failed".into() });
        // The evidence it earned before dying is still recorded.
        assert_eq!(read_evidence(&db, "run-1").unwrap().len(), 1);
    }

    #[test]
    fn a_provider_that_never_stops_asking_is_stopped_by_its_turn_ceiling() {
        // An unattended run whose provider loops must end, not spin.
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![]);
        struct Looping;
        impl BriefingProvider for Looping {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<(), String> { Ok(()) }
            fn pending_calls(&mut self) -> Vec<ToolCall> { vec![call("c", SLACK_TOOL)] }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult, _earned: Option<&str>) {}
            fn answer(&mut self) -> Result<String, String> { Ok(EMPTY_BRIEF.to_owned()) }
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Looping).unwrap();
        assert_eq!(result, RunResult::Failed { run_id: "run-1".into(), code: "budget_exceeded".into() });
    }

    #[test]
    fn a_run_with_nothing_eligible_explains_itself_rather_than_looking_untried() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors::with(vec![]);
        connectors.instances.retain(|instance| instance.instance_id != "slack-1");
        let mut provider = Citing::new(vec![]);
        run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        let coverage = read_coverage(&db, "run-1").unwrap();
        assert_eq!(coverage.len(), 2);
        for row in &coverage {
            assert!(row.detail.is_some(), "each ineligible source says why");
        }
    }

    #[test]
    fn a_skipped_run_carries_the_configurations_code() {
        assert_eq!(
            skip(&BriefingUnavailable::NotConfigured),
            RunResult::Skipped { code: "not_configured".into() }
        );
        assert_eq!(
            skip(&BriefingUnavailable::ProviderUnsupported { harness: "codex".into(), reason: "r".into() }),
            RunResult::Skipped { code: "provider_unsupported".into() }
        );
    }
}
