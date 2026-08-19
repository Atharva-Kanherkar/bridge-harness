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
use crate::work_brief_store::{begin_run, finish_run, record_ledger, RunOutcome, RunStart};
use crate::work_briefing_config::{BriefingSelection, BriefingUnavailable};
use crate::work_connectors::{eligibility, ConnectorEligibility, ConnectorInstance};
use crate::work_evidence::RunLedger;
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
    /// Start the run and return the provider's first message.
    fn open(&mut self, offered: &[BriefingToolIdentity]) -> Result<String, String>;
    /// The calls the provider wants to make before it answers. Called until empty.
    fn pending_calls(&mut self) -> Vec<ToolCall>;
    /// Hand a call's outcome back, so the provider can continue.
    fn deliver(&mut self, call_id: &str, outcome: &ToolResult);
    /// Ask for one repair, given why the last answer was refused. `None` if the provider
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
    /// A brief was accepted. Committing its tasks is slice 5's job — this slice
    /// deliberately stops here, with the evidence recorded and nothing on the board
    /// changed.
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

    let first = match provider.open(&offered) {
        Ok(message) => message,
        Err(detail) => {
            return finish_failed(db, &mut ledger, &request, "provider_failed", Some(detail), provider)
        }
    };

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
                    provider.deliver(&call.call_id, &ToolResult::Failed { detail: "unknown tool".into() });
                    continue;
                }
            };
            if let ToolDecision::Deny(denial) = policy.decide(&call.tool, bytes) {
                // Denied: the model tried, which is worth recording, but nothing is
                // earned and no connector is touched.
                ledger.record_consulted(&instance.instance_id, family.as_str());
                provider.deliver(&call.call_id, &ToolResult::Failed { detail: denial.reason() });
                continue;
            }
            ledger.record_consulted(&instance.instance_id, family.as_str());
            let outcome = connectors.call(&call);
            match &outcome {
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
                    );
                }
                ToolResult::Failed { detail } => {
                    ledger.record_failed(&instance.instance_id, family.as_str(), detail.clone())
                }
            }
            provider.deliver(&call.call_id, &outcome);
        }
    }

    // Read the answer, with exactly one repair.
    let outcome = parse_with_one_repair(&first, &ledger, |rejection| provider.repair(rejection));
    record_ledger(db, &ledger)?;

    match outcome {
        BriefOutcome::Accepted { brief, repaired } => {
            finish_run(
                db,
                request.run_id,
                &RunOutcome {
                    status: wire::WorkBriefRunStatus::Succeeded,
                    output_digest: Some(output_digest(&brief)),
                    failure_code: None,
                    failure_detail: None,
                    usage: provider.usage(),
                    tool_calls: guard.tool_calls(),
                    turns,
                    completed_at: (request.now)(),
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
            finish_run(
                db,
                request.run_id,
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
    record_ledger(db, ledger)?;
    finish_run(
        db,
        request.run_id,
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

    fn definition() -> serde_json::Value {
        json!({"name": "search", "parameters": {"query": {"type": "string"}}})
    }

    /// A signed-in Slack instance with one reviewed tool, plus a Gmail instance nobody
    /// is signed into and a Notion instance with no reviewed tools.
    fn instances() -> Vec<ConnectorInstance> {
        let reviewed = identity("slack-1", "search_messages");
        let digest = tool_definition_digest(&definition());
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
        /// What the next call returns, in order.
        results: Vec<ToolResult>,
        calls: Vec<String>,
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

    struct Provider {
        calls: Vec<Vec<ToolCall>>,
        /// The answer, built once the run knows what references exist.
        answer: Box<dyn Fn() -> String>,
        repair: Option<String>,
        repairs_asked: std::cell::Cell<usize>,
        delivered: std::cell::RefCell<Vec<(String, bool)>>,
        offered: std::cell::RefCell<Vec<String>>,
    }

    impl BriefingProvider for Provider {
        fn open(&mut self, offered: &[BriefingToolIdentity]) -> Result<String, String> {
            *self.offered.borrow_mut() = offered.iter().map(BriefingToolIdentity::wire_name).collect();
            Ok(String::new())
        }
        fn pending_calls(&mut self) -> Vec<ToolCall> {
            if self.calls.is_empty() { vec![] } else { self.calls.remove(0) }
        }
        fn deliver(&mut self, call_id: &str, outcome: &ToolResult) {
            self.delivered
                .borrow_mut()
                .push((call_id.to_owned(), matches!(outcome, ToolResult::Succeeded(_))));
        }
        fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> {
            self.repairs_asked.set(self.repairs_asked.get() + 1);
            self.repair.clone()
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

    fn brief_citing(reference: &str) -> String {
        format!(
            "```bridge-work-brief\n{}\n```",
            format!(
                r#"{{"version":1,"tasks":[{{"rank":1,"title":"Reply to Priya","why":"Asked twice.","confidenceBps":8200,"evidence":["{reference}"]}}]}}"#
            )
        )
    }

    fn request<'a>(run_id: &'a str, selection: &'a BriefingSelection, now: &'a dyn Fn() -> String) -> RunRequest<'a> {
        RunRequest {
            run_id,
            trigger: wire::WorkBriefTrigger::Manual,
            selection,
            limits: limits(),
            session_id: None,
            idempotency_key: None,
            started_at: STARTED,
            now,
        }
    }

    #[test]
    fn a_whole_run_offers_reads_parses_and_records() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors {
            instances: instances(),
            results: vec![ToolResult::Succeeded(slack_result())],
            calls: vec![],
        };
        let mut provider = Provider {
            calls: vec![vec![call("c1", "mcp__slack-1__search_messages")]],
            // The reference the run will have earned by the time it answers.
            answer: Box::new(|| brief_citing("run-1:ev-1")),
            repair: None,
            repairs_asked: std::cell::Cell::new(0),
            delivered: std::cell::RefCell::new(vec![]),
            offered: std::cell::RefCell::new(vec![]),
        };
        // The provider answers on the first message, so stage it as the opening text.
        let answer = (provider.answer)();
        struct Answering<'a>(&'a mut Provider, String);
        impl BriefingProvider for Answering<'_> {
            fn open(&mut self, offered: &[BriefingToolIdentity]) -> Result<String, String> {
                self.0.open(offered)?;
                Ok(self.1.clone())
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> { self.0.pending_calls() }
            fn deliver(&mut self, id: &str, outcome: &ToolResult) { self.0.deliver(id, outcome) }
            fn repair(&mut self, rejection: &BriefRejection) -> Option<String> { self.0.repair(rejection) }
            fn usage(&self) -> Option<wire::WorkRunUsage> { self.0.usage() }
        }
        let mut answering = Answering(&mut provider, answer);

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut answering).unwrap();
        let RunResult::Accepted { brief, repaired, .. } = &result else {
            panic!("expected an accepted run, got {result:?}");
        };
        assert!(!repaired);
        assert_eq!(brief.tasks.len(), 1);

        // Only the eligible connector was offered.
        assert_eq!(*answering.0.offered.borrow(), vec!["mcp__slack-1__search_messages"]);

        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Succeeded);
        assert!(run.output_digest.is_some(), "an accepted brief is digested, not stored");
        assert_eq!(run.profile_reference.as_deref(), Some("claude/sonnet/medium"));

        // Coverage tells the three connectors apart.
        let coverage = read_coverage(&db, "run-1").unwrap();
        let status = |id: &str| coverage.iter().find(|row| row.connector_instance_id == id).unwrap().status;
        assert_eq!(status("slack-1"), wire::WorkSourceStatus::Succeeded);
        assert_eq!(status("gmail-1"), wire::WorkSourceStatus::AuthRequired, "not a failure");
        assert_eq!(status("notion-1"), wire::WorkSourceStatus::Ineligible);

        let evidence = read_evidence(&db, "run-1").unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].canonical_resource_id, "slack:slack-1:1723459200.123");

        // The board is untouched: committing tasks is slice 5.
        let tasks: i64 = db.query_row("SELECT COUNT(*) FROM work_tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(tasks, 0);
        let facts: i64 = db.query_row("SELECT COUNT(*) FROM work_fact_cache", [], |row| row.get(0)).unwrap();
        assert_eq!(facts, 0, "the facts board is not touched by a briefing");
    }

    #[test]
    fn a_fabricated_citation_fails_the_run_after_exactly_one_repair() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors { instances: instances(), results: vec![], calls: vec![] };
        struct Fabricating {
            repairs: std::cell::Cell<usize>,
        }
        impl BriefingProvider for Fabricating {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                // Cites evidence no call earned.
                Ok(format!(
                    "```bridge-work-brief\n{}\n```",
                    r#"{"version":1,"tasks":[{"rank":1,"title":"t","why":"w","confidenceBps":100,"evidence":["run-1:ev-99"]}]}"#
                ))
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> { vec![] }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> {
                self.repairs.set(self.repairs.get() + 1);
                // Fabricates again.
                Some(format!(
                    "```bridge-work-brief\n{}\n```",
                    r#"{"version":1,"tasks":[{"rank":1,"title":"t","why":"w","confidenceBps":100,"evidence":["run-1:ev-98"]}]}"#
                ))
            }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let mut provider = Fabricating { repairs: std::cell::Cell::new(0) };

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut provider).unwrap();
        assert_eq!(
            result,
            RunResult::Failed { run_id: "run-1".into(), code: "evidence_invalid".into() }
        );
        assert_eq!(provider.repairs.get(), 1, "exactly one repair, however many would fail");

        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Failed);
        assert_eq!(run.failure_code.as_deref(), Some("evidence_invalid"));
        assert!(run.output_digest.is_none(), "nothing was accepted, so nothing is digested");
        // The useful part of a failed run survives: what was offered and why.
        assert_eq!(read_coverage(&db, "run-1").unwrap().len(), 3);
    }

    #[test]
    fn a_denied_call_is_recorded_as_consulted_and_touches_no_connector() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors { instances: instances(), results: vec![], calls: vec![] };
        struct Escalating(bool);
        impl BriefingProvider for Escalating {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                Ok(format!("```bridge-work-brief\n{}\n```", r#"{"version":1,"tasks":[]}"#))
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> {
                if self.0 { return vec![] }
                self.0 = true;
                // Asks for the reviewed tool's neighbour, which nobody reviewed.
                vec![call("c1", "mcp__slack-1__post_message")]
            }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }

        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Escalating(false)).unwrap();
        assert!(matches!(result, RunResult::Accepted { .. }), "an empty brief is a valid answer");
        // The connector was never called: an unreviewed tool belongs to no instance the
        // runner will serve.
        assert!(connectors.calls.is_empty(), "no connector is touched for a tool nobody reviewed");
        assert!(read_evidence(&db, "run-1").unwrap().is_empty());
    }

    #[test]
    fn a_failed_connector_call_earns_no_evidence_but_is_recorded() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors {
            instances: instances(),
            results: vec![ToolResult::Failed { detail: "503 from Slack".into() }],
            calls: vec![],
        };
        struct Trying(bool);
        impl BriefingProvider for Trying {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                Ok(format!("```bridge-work-brief\n{}\n```", r#"{"version":1,"tasks":[]}"#))
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> {
                if self.0 { return vec![] }
                self.0 = true;
                vec![call("c1", "mcp__slack-1__search_messages")]
            }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }

        run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Trying(false)).unwrap();
        assert_eq!(connectors.calls.len(), 1, "the allowed call did reach the connector");
        assert!(read_evidence(&db, "run-1").unwrap().is_empty(), "a failure vouches for nothing");
        let coverage = read_coverage(&db, "run-1").unwrap();
        let slack = coverage.iter().find(|row| row.connector_instance_id == "slack-1").unwrap();
        assert_eq!(slack.status, wire::WorkSourceStatus::Failed);
        assert_eq!(slack.detail.as_deref(), Some("503 from Slack"));
    }

    #[test]
    fn a_provider_that_never_opens_still_leaves_a_run_to_ask_about() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors { instances: instances(), results: vec![], calls: vec![] };
        struct Broken;
        impl BriefingProvider for Broken {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                Err("the sidecar exited before answering".into())
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> { vec![] }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Broken).unwrap();
        assert_eq!(result, RunResult::Failed { run_id: "run-1".into(), code: "provider_failed".into() });
        // A run that vanished is one nobody can ask about.
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Failed);
        assert_eq!(read_coverage(&db, "run-1").unwrap().len(), 3);
    }

    #[test]
    fn a_run_with_nothing_eligible_explains_itself_rather_than_looking_untried() {
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors {
            instances: instances().into_iter().filter(|i| i.instance_id != "slack-1").collect(),
            results: vec![],
            calls: vec![],
        };
        struct Quiet;
        impl BriefingProvider for Quiet {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                Ok(format!("```bridge-work-brief\n{}\n```", r#"{"version":1,"tasks":[]}"#))
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> { vec![] }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Quiet).unwrap();
        let coverage = read_coverage(&db, "run-1").unwrap();
        assert_eq!(coverage.len(), 2);
        for row in &coverage {
            assert!(row.detail.is_some(), "each ineligible source says why");
        }
    }

    #[test]
    fn a_provider_that_never_stops_asking_is_stopped_by_its_turn_ceiling() {
        // Found while writing the test above: a stub that kept requesting calls ran until
        // the guard refused a turn. That is the protection working, and it is worth
        // pinning — an unattended run whose provider loops must end, not spin.
        let mut db = db();
        let chosen = selection();
        let now = || STARTED.to_owned();
        let mut connectors = Connectors {
            instances: instances(),
            results: vec![],
            calls: vec![],
        };
        struct Looping;
        impl BriefingProvider for Looping {
            fn open(&mut self, _offered: &[BriefingToolIdentity]) -> Result<String, String> {
                Ok(String::new())
            }
            fn pending_calls(&mut self) -> Vec<ToolCall> {
                vec![call("c", "mcp__slack-1__search_messages")]
            }
            fn deliver(&mut self, _id: &str, _outcome: &ToolResult) {}
            fn repair(&mut self, _rejection: &BriefRejection) -> Option<String> { None }
            fn usage(&self) -> Option<wire::WorkRunUsage> { None }
        }
        let result = run_briefing(&mut db, request("run-1", &chosen, &now), &mut connectors, &mut Looping).unwrap();
        assert_eq!(
            result,
            RunResult::Failed { run_id: "run-1".into(), code: "budget_exceeded".into() }
        );
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.failure_code.as_deref(), Some("budget_exceeded"));
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
