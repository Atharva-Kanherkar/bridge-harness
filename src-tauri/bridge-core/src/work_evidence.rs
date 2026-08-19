//! The run-local evidence ledger, and what each source got up to.
//!
//! A briefing's output is only worth committing if every claim in it can be traced to
//! something Bridge watched happen. This module is that record.
//!
//! One rule decides its whole shape: **a reference is earned, never granted.** It
//! exists because a call was allowed by the policy, belonged to this run, and returned
//! successfully. A failed call earns nothing, a denied call earns nothing, and another
//! run's evidence is not in this run's ledger — so the parser's citation check refuses
//! all three without knowing the difference between them.
//!
//! The other rule is about what gets stored: **digests and Bridge-derived identity, no
//! payloads.** A connector result is untrusted text that may be large, may contain
//! secrets, and is not something a board needs. What a board needs is to know the
//! result was real and to be able to recognise the same thing again.

use std::collections::{BTreeMap, BTreeSet};

use bridge_protocol::messages as wire;
use sha2::{Digest, Sha256};

use crate::work_brief_parser::EvidenceLedger;
use crate::work_connectors::{resolve_evidence, ConnectorFamily, EvidenceTarget, ResolvedEvidence};

/// One earned piece of evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceEntry {
    /// The reference a brief cites. Namespaced by run, so a citation from another run
    /// cannot resolve here even by coincidence.
    pub evidence_ref: String,
    /// The provider's id for the call that earned this, kept so a transcript can be
    /// lined up with the ledger.
    pub tool_call_id: String,
    pub connector_instance_id: String,
    pub account_identity: Option<String>,
    pub canonical_resource_id: String,
    pub source_kind: String,
    pub tool_definition_digest: String,
    /// A digest of the result, not the result. Enough to show it did not change under
    /// us; nothing anybody could read back out.
    pub result_digest: String,
    pub target: EvidenceTarget,
    /// When Bridge saw the result, not when the connector says the thing happened.
    pub observed_at: String,
}

/// Digest a connector result. Prefixed and versioned so a digest cannot be mistaken
/// for one computed over something else.
pub fn result_digest(result: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"bridge-connector-result-v1\0");
    hasher.update(serde_json::to_string(result).unwrap_or_default().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// One connector instance's progress through a run.
///
/// The six states are separate facts, and the ledger tracks the furthest each source
/// reached. "Available" is not "was read": a run that offered three sources, had the
/// model call two, and got one useful answer must be readable as exactly that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCoverage {
    pub connector_instance_id: String,
    pub connector_family: String,
    pub status: wire::WorkSourceStatus,
    pub detail: Option<String>,
    pub observed_at: Option<String>,
}

/// How far along a status is, so a source's furthest point is what gets recorded.
///
/// Deliberately not a simple "latest wins": a failed call after a successful one must
/// not erase the fact that this source produced usable evidence, and a source that
/// succeeded once has succeeded.
fn progress(status: wire::WorkSourceStatus) -> u8 {
    match status {
        wire::WorkSourceStatus::Ineligible => 0,
        wire::WorkSourceStatus::AuthRequired => 1,
        wire::WorkSourceStatus::Eligible => 2,
        wire::WorkSourceStatus::Consulted => 3,
        wire::WorkSourceStatus::Failed => 4,
        wire::WorkSourceStatus::Succeeded => 5,
    }
}

/// The ledger for one run.
#[derive(Debug, Clone)]
pub struct RunLedger {
    run_id: String,
    entries: Vec<EvidenceEntry>,
    refs: BTreeSet<String>,
    resources: BTreeSet<String>,
    coverage: BTreeMap<String, SourceCoverage>,
    next_index: usize,
}

impl RunLedger {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            entries: Vec::new(),
            refs: BTreeSet::new(),
            resources: BTreeSet::new(),
            coverage: BTreeMap::new(),
            next_index: 1,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Note that a source was offered, refused, or needs signing in. Called before the
    /// model runs, from `work_connectors::eligibility`.
    pub fn record_source(
        &mut self,
        instance_id: &str,
        family: &str,
        status: wire::WorkSourceStatus,
        detail: Option<String>,
        observed_at: Option<String>,
    ) {
        let entry = self.coverage.entry(instance_id.to_owned()).or_insert_with(|| SourceCoverage {
            connector_instance_id: instance_id.to_owned(),
            connector_family: family.to_owned(),
            status,
            detail: detail.clone(),
            observed_at: observed_at.clone(),
        });
        // One row per connector per run, holding the furthest point it reached.
        if progress(status) >= progress(entry.status) {
            entry.status = status;
            entry.detail = detail;
            if observed_at.is_some() {
                entry.observed_at = observed_at;
            }
        }
    }

    /// A call the model made. Recorded whatever it returns, because "the model called
    /// it" is a fact worth having even when the call failed.
    pub fn record_consulted(&mut self, instance_id: &str, family: &str) {
        self.record_source(instance_id, family, wire::WorkSourceStatus::Consulted, None, None);
    }

    /// A call that failed. Earns no reference.
    pub fn record_failed(&mut self, instance_id: &str, family: &str, detail: impl Into<String>) {
        self.record_source(
            instance_id,
            family,
            wire::WorkSourceStatus::Failed,
            Some(detail.into()),
            None,
        );
    }

    /// A call that succeeded: derive provenance and earn a reference.
    ///
    /// Returns the reference the model may cite, or `None` when Bridge could not derive
    /// a canonical id — evidence it cannot recognise again is evidence it does not keep.
    /// The caller must only reach here for a call the policy allowed; a denied call has
    /// no result to record.
    pub fn record_succeeded(
        &mut self,
        family: ConnectorFamily,
        instance_id: &str,
        account_identity: Option<&str>,
        tool_call_id: &str,
        tool_definition_digest: &str,
        result: &serde_json::Value,
        observed_at: &str,
    ) -> Option<String> {
        let ResolvedEvidence {
            canonical_resource_id,
            source_kind,
            target,
        } = resolve_evidence(family, instance_id, result)?;

        // The same resource read twice in one run is one piece of evidence. Returning
        // the existing reference rather than a second one keeps a brief from citing the
        // same thing twice under two names.
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.canonical_resource_id == canonical_resource_id)
        {
            let reused = existing.evidence_ref.clone();
            self.record_source(
                instance_id,
                family.as_str(),
                wire::WorkSourceStatus::Succeeded,
                None,
                Some(observed_at.to_owned()),
            );
            return Some(reused);
        }

        let evidence_ref = format!("{}:ev-{}", self.run_id, self.next_index);
        self.next_index += 1;
        self.refs.insert(evidence_ref.clone());
        self.resources.insert(canonical_resource_id.clone());
        self.entries.push(EvidenceEntry {
            evidence_ref: evidence_ref.clone(),
            tool_call_id: tool_call_id.to_owned(),
            connector_instance_id: instance_id.to_owned(),
            account_identity: account_identity.map(str::to_owned),
            canonical_resource_id,
            source_kind,
            tool_definition_digest: tool_definition_digest.to_owned(),
            result_digest: result_digest(result),
            target,
            observed_at: observed_at.to_owned(),
        });
        self.record_source(
            instance_id,
            family.as_str(),
            wire::WorkSourceStatus::Succeeded,
            None,
            Some(observed_at.to_owned()),
        );
        Some(evidence_ref)
    }

    pub fn entries(&self) -> &[EvidenceEntry] {
        &self.entries
    }

    pub fn entry(&self, evidence_ref: &str) -> Option<&EvidenceEntry> {
        self.entries.iter().find(|entry| entry.evidence_ref == evidence_ref)
    }

    /// Coverage rows, ordered by connector so a report does not depend on call order.
    pub fn coverage(&self) -> Vec<SourceCoverage> {
        self.coverage.values().cloned().collect()
    }

    /// How many sources reached at least this far. The accounting the run reports.
    pub fn count_at_least(&self, status: wire::WorkSourceStatus) -> usize {
        self.coverage
            .values()
            .filter(|entry| progress(entry.status) >= progress(status))
            .count()
    }

    /// Sources that reached exactly this state.
    pub fn count_exactly(&self, status: wire::WorkSourceStatus) -> usize {
        self.coverage.values().filter(|entry| entry.status == status).count()
    }
}

/// The parser asks one question, and this is the answer for this run.
impl EvidenceLedger for RunLedger {
    fn contains(&self, evidence_ref: &str) -> bool {
        self.refs.contains(evidence_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_brief_parser::{parse_brief, BriefRejection};
    use serde_json::json;

    const SEEN: &str = "2026-08-19T12:00:00+00:00";

    fn slack_result() -> serde_json::Value {
        json!({
            "ts": "1723459200.123",
            "permalink": "https://app.slack.com/archives/C1/p1723459200123",
            "text": "sk-ant-a-secret-that-must-not-be-stored",
        })
    }

    fn ledger_with_one_success() -> (RunLedger, String) {
        let mut ledger = RunLedger::new("run-7");
        let reference = ledger
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                Some("T0001/U0001"),
                "call_1",
                "digest-abc",
                &slack_result(),
                SEEN,
            )
            .expect("a slack result with a ts earns a reference");
        (ledger, reference)
    }

    // -----------------------------------------------------------------------
    // A reference is earned
    // -----------------------------------------------------------------------

    #[test]
    fn only_a_successful_allowlisted_call_earns_a_reference() {
        let (ledger, reference) = ledger_with_one_success();
        assert!(ledger.contains(&reference));
        assert_eq!(ledger.entries().len(), 1);
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Succeeded), 1);
    }

    #[test]
    fn a_failed_call_earns_nothing() {
        let mut ledger = RunLedger::new("run-7");
        ledger.record_consulted("slack-1", "slack");
        ledger.record_failed("slack-1", "slack", "the connector returned 503");
        assert!(ledger.entries().is_empty(), "a failure has no result to vouch for");
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Failed), 1);
        // And nothing can be cited, which is what makes the parser refuse a brief that
        // tries to cite it.
        assert!(!ledger.contains("run-7:ev-1"));
    }

    #[test]
    fn a_denied_call_earns_nothing() {
        // A denied call never reaches record_succeeded, so the ledger stays empty and
        // the source's furthest point is that the model tried.
        let mut ledger = RunLedger::new("run-7");
        ledger.record_consulted("slack-1", "slack");
        assert!(ledger.entries().is_empty());
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Consulted), 1);
    }

    #[test]
    fn a_result_bridge_cannot_identify_earns_nothing() {
        let mut ledger = RunLedger::new("run-7");
        let earned = ledger.record_succeeded(
            ConnectorFamily::Slack,
            "slack-1",
            None,
            "call_1",
            "digest-abc",
            &json!({"text": "no ts here"}),
            SEEN,
        );
        assert!(earned.is_none(), "evidence with no canonical id is not kept");
        assert!(ledger.entries().is_empty());
    }

    #[test]
    fn references_are_unique_within_a_run() {
        let mut ledger = RunLedger::new("run-7");
        let first = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-1", None, "c1", "d", &json!({"ts": "1.1"}), SEEN)
            .unwrap();
        let second = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-1", None, "c2", "d", &json!({"ts": "2.2"}), SEEN)
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(ledger.entries().len(), 2);
    }

    #[test]
    fn the_same_resource_read_twice_is_one_piece_of_evidence() {
        // Otherwise a brief could cite the same message twice under two names and look
        // like it had two sources.
        let mut ledger = RunLedger::new("run-7");
        let first = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-1", None, "c1", "d", &slack_result(), SEEN)
            .unwrap();
        let again = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-1", None, "c2", "d", &slack_result(), SEEN)
            .unwrap();
        assert_eq!(first, again);
        assert_eq!(ledger.entries().len(), 1);
    }

    #[test]
    fn evidence_is_scoped_to_its_run() {
        let (seven, reference) = ledger_with_one_success();
        let eight = RunLedger::new("run-8");
        assert!(seven.contains(&reference));
        assert!(!eight.contains(&reference), "another run has not earned this");
        assert!(reference.starts_with("run-7:"), "the run is part of the reference");
    }

    #[test]
    fn a_brief_citing_another_runs_evidence_is_refused_by_the_parser() {
        // The two halves meeting: the ledger only holds what this run earned, and the
        // parser refuses a citation it cannot find. Neither knows about the other's
        // rules.
        let (ledger, _) = ledger_with_one_success();
        let payload = format!(
            "```bridge-work-brief\n{}\n```",
            r#"{"version":1,"tasks":[{"rank":1,"title":"t","why":"w","confidenceBps":100,"evidence":["run-6:ev-1"]}]}"#
        );
        assert_eq!(
            parse_brief(&payload, &ledger).unwrap_err(),
            BriefRejection::EvidenceUnknown
        );
    }

    #[test]
    fn a_brief_citing_this_runs_evidence_parses() {
        let (ledger, reference) = ledger_with_one_success();
        let payload = format!(
            "```bridge-work-brief\n{}\n```",
            format!(
                r#"{{"version":1,"tasks":[{{"rank":1,"title":"t","why":"w","confidenceBps":100,"evidence":["{reference}"]}}]}}"#
            )
        );
        assert!(parse_brief(&payload, &ledger).is_ok());
    }

    // -----------------------------------------------------------------------
    // What is stored
    // -----------------------------------------------------------------------

    #[test]
    fn the_ledger_stores_digests_and_never_a_payload() {
        let (ledger, reference) = ledger_with_one_success();
        let entry = ledger.entry(&reference).unwrap();
        assert_eq!(entry.result_digest.len(), 64, "a sha256, not the result");
        assert_eq!(entry.tool_definition_digest, "digest-abc");
        // The result carried a secret. Nothing rendered from the entry may contain it.
        let rendered = format!("{entry:?}");
        assert!(!rendered.contains("sk-ant"), "no payload text is retained");
        assert!(!rendered.contains("a-secret-that-must-not-be-stored"));
    }

    #[test]
    fn provenance_on_an_entry_is_all_bridge_derived() {
        let (ledger, reference) = ledger_with_one_success();
        let entry = ledger.entry(&reference).unwrap();
        assert_eq!(entry.canonical_resource_id, "slack:slack-1:1723459200.123");
        assert_eq!(entry.account_identity.as_deref(), Some("T0001/U0001"));
        assert_eq!(entry.observed_at, SEEN, "when Bridge saw it, not what the result claims");
        assert_eq!(entry.tool_call_id, "call_1");
        assert_eq!(
            entry.target,
            EvidenceTarget::ExternalLink {
                url: "https://app.slack.com/archives/C1/p1723459200123".into(),
                host: "app.slack.com".into(),
            }
        );
    }

    #[test]
    fn a_result_claiming_its_own_provenance_is_ignored() {
        let mut ledger = RunLedger::new("run-7");
        let reference = ledger
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                Some("real-account"),
                "call_1",
                "digest-abc",
                &json!({
                    "ts": "1.1",
                    "canonicalResourceId": "attacker-chosen",
                    "accountIdentity": "someone-else",
                    "observedAt": "1999-01-01T00:00:00+00:00",
                    "permalink": "https://evil.example/x",
                }),
                SEEN,
            )
            .unwrap();
        let entry = ledger.entry(&reference).unwrap();
        assert_eq!(entry.canonical_resource_id, "slack:slack-1:1.1");
        assert_eq!(entry.account_identity.as_deref(), Some("real-account"));
        assert_eq!(entry.observed_at, SEEN);
        assert_eq!(entry.target, EvidenceTarget::None, "an off-allowlist link is no link");
    }

    // -----------------------------------------------------------------------
    // Coverage is six separate facts
    // -----------------------------------------------------------------------

    #[test]
    fn the_six_states_are_tracked_independently() {
        let mut ledger = RunLedger::new("run-7");
        ledger.record_source("a", "slack", wire::WorkSourceStatus::Ineligible, Some("no resolver".into()), None);
        ledger.record_source("b", "gmail", wire::WorkSourceStatus::AuthRequired, None, None);
        ledger.record_source("c", "github", wire::WorkSourceStatus::Eligible, None, None);
        ledger.record_consulted("d", "linear");
        ledger.record_failed("e", "notion", "503");
        ledger
            .record_succeeded(ConnectorFamily::Slack, "f", None, "c1", "d", &json!({"ts": "1.1"}), SEEN)
            .unwrap();

        for status in [
            wire::WorkSourceStatus::Ineligible,
            wire::WorkSourceStatus::AuthRequired,
            wire::WorkSourceStatus::Eligible,
            wire::WorkSourceStatus::Consulted,
            wire::WorkSourceStatus::Failed,
            wire::WorkSourceStatus::Succeeded,
        ] {
            assert_eq!(ledger.count_exactly(status), 1, "{status:?} is its own fact");
        }
        assert_eq!(ledger.coverage().len(), 6);
    }

    #[test]
    fn consulted_is_not_succeeded() {
        // "Available" is not "was read", and "was read" is not "answered usefully".
        let mut ledger = RunLedger::new("run-7");
        ledger.record_source("a", "slack", wire::WorkSourceStatus::Eligible, None, None);
        ledger.record_consulted("a", "slack");
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Consulted), 1);
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Succeeded), 0);
        assert_eq!(ledger.count_at_least(wire::WorkSourceStatus::Eligible), 1);
    }

    #[test]
    fn an_auth_required_source_is_not_a_failure() {
        let mut ledger = RunLedger::new("run-7");
        ledger.record_source("a", "gmail", wire::WorkSourceStatus::AuthRequired, None, None);
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Failed), 0);
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::AuthRequired), 1);
    }

    #[test]
    fn coverage_is_one_row_per_connector_per_run() {
        let mut ledger = RunLedger::new("run-7");
        ledger.record_source("a", "slack", wire::WorkSourceStatus::Eligible, None, None);
        ledger.record_consulted("a", "slack");
        ledger.record_consulted("a", "slack");
        assert_eq!(ledger.coverage().len(), 1);
    }

    #[test]
    fn a_later_failure_does_not_erase_a_success() {
        // A source that produced usable evidence produced it, whatever a second call
        // did afterwards.
        let mut ledger = RunLedger::new("run-7");
        ledger
            .record_succeeded(ConnectorFamily::Slack, "a", None, "c1", "d", &json!({"ts": "1.1"}), SEEN)
            .unwrap();
        ledger.record_failed("a", "slack", "the second call timed out");
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Succeeded), 1);
        assert_eq!(ledger.count_exactly(wire::WorkSourceStatus::Failed), 0);
    }

    #[test]
    fn a_source_never_goes_backwards() {
        let mut ledger = RunLedger::new("run-7");
        ledger.record_consulted("a", "slack");
        ledger.record_source("a", "slack", wire::WorkSourceStatus::Eligible, None, None);
        assert_eq!(
            ledger.count_exactly(wire::WorkSourceStatus::Consulted),
            1,
            "being offered again does not undo having been called"
        );
    }

    #[test]
    fn coverage_order_does_not_depend_on_call_order() {
        let mut one = RunLedger::new("run-7");
        one.record_consulted("z", "slack");
        one.record_consulted("a", "gmail");
        let mut other = RunLedger::new("run-7");
        other.record_consulted("a", "gmail");
        other.record_consulted("z", "slack");
        let ids = |ledger: &RunLedger| {
            ledger.coverage().into_iter().map(|row| row.connector_instance_id).collect::<Vec<_>>()
        };
        assert_eq!(ids(&one), ids(&other));
        assert_eq!(ids(&one), vec!["a", "z"]);
    }
}
