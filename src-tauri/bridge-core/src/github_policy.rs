//! The per-action approval gate for mutating GitHub operations.
//!
//! Every mutation on the GitHub surface — merge, review, reply, re-run, label,
//! comment, close/reopen, ready-for-review — is an approval-tier operation: it
//! runs only after the user confirms the exact statement of what will happen. This module owns two things and nothing else:
//! the decision (approve vs deny) and the canonical statement shown in the
//! confirmation. It never spawns a subprocess and never touches `gh`; the api
//! layer consults it *before* the surface is reached, so a denial provably
//! results in zero subprocess calls.
//!
//! This is deliberately separate from the delegation policy engine. The two
//! solve different problems — delegation routing decides worker capability and
//! write scope; this decides whether one already-described GitHub write may
//! proceed — and conflating them is exactly the hierarchy-mixing bug the
//! architecture warns against.

use crate::github_surface::{
    review_label, strategy_label, GithubAction, LabelOperation, StateOperation,
};

/// The outcome of the approval gate for a single action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubActionDecision {
    /// The user confirmed; the action may execute.
    Approved,
    /// The user declined; nothing executes and no subprocess is spawned.
    Denied,
}

impl GithubActionDecision {
    pub fn is_approved(self) -> bool {
        matches!(self, GithubActionDecision::Approved)
    }
}

/// Map a native confirmation outcome to a decision. `confirmed` is the result
/// of the user's per-action approval; there is no implicit approval path.
pub fn authorize(confirmed: bool) -> GithubActionDecision {
    if confirmed {
        GithubActionDecision::Approved
    } else {
        GithubActionDecision::Denied
    }
}

/// The operation-and-target phrase for an action, without the repository, e.g.
/// `merge PR #328 (squash)`. The declined-outcome message is built from this so
/// the backend can name a refused action without resolving the repository (a
/// resolution that would itself spawn `gh` — never acceptable on a denial).
pub fn summary(action: &GithubAction) -> String {
    let number = action.number();
    match action {
        GithubAction::Merge { strategy, .. } => {
            format!("merge PR #{number} ({})", strategy_label(*strategy))
        }
        GithubAction::Review { event, .. } => {
            format!("submit {} on PR #{number}", review_label(*event))
        }
        GithubAction::Reply { .. } => format!("reply to a review comment on PR #{number}"),
        GithubAction::Rerun { .. } => format!("re-run failed checks on PR #{number}"),
        GithubAction::Label { target, label, operation, .. } => format!(
            "{} label {label:?} {} {} #{number}",
            match operation { LabelOperation::Add => "add", LabelOperation::Remove => "remove" },
            match operation { LabelOperation::Add => "to", LabelOperation::Remove => "from" },
            target.label(),
        ),
        GithubAction::Comment { target, .. } => {
            format!("post a comment on {} #{number}", target.label())
        }
        GithubAction::SetState { target, operation, .. } => format!(
            "{} {} #{number}",
            match operation { StateOperation::Close => "close", StateOperation::Reopen => "reopen" },
            target.label(),
        ),
        GithubAction::Ready { .. } => format!("mark PR #{number} ready for review"),
    }
}

/// The exact operation-and-target statement for an action, e.g.
/// `merge PR #328 (squash) on owner/repo`. The confirmation chrome and the
/// backend audit are both built from this so what the user sees and what the
/// backend records cannot drift.
pub fn describe(action: &GithubAction, repository: &str) -> String {
    format!("{} on {repository}", summary(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_surface::{GithubTarget, MergeStrategy, ReviewEvent, StateOperation};

    #[test]
    fn a_confirmed_action_authorizes_to_approved() {
        assert_eq!(authorize(true), GithubActionDecision::Approved);
        assert!(authorize(true).is_approved());
    }

    #[test]
    fn a_denied_action_authorizes_to_denied() {
        assert_eq!(authorize(false), GithubActionDecision::Denied);
        assert!(!authorize(false).is_approved());
    }

    #[test]
    fn describe_states_the_exact_operation_and_target() {
        let merge = GithubAction::Merge {
            number: 328,
            strategy: MergeStrategy::Squash,
        };
        assert_eq!(describe(&merge, "owner/repo"), "merge PR #328 (squash) on owner/repo");

        let approve = GithubAction::Review {
            number: 341,
            event: ReviewEvent::Approve,
            body: String::new(),
        };
        assert_eq!(describe(&approve, "owner/repo"), "submit approval on PR #341 on owner/repo");

        let reply = GithubAction::Reply {
            number: 341,
            comment_id: 7,
            body: "thanks".into(),
        };
        assert_eq!(describe(&reply, "owner/repo"), "reply to a review comment on PR #341 on owner/repo");

        let rerun = GithubAction::Rerun { number: 104 };
        assert_eq!(describe(&rerun, "owner/repo"), "re-run failed checks on PR #104 on owner/repo");

        let label = GithubAction::Label {
            target: GithubTarget::Issue,
            number: 9,
            label: "bug".into(),
            operation: LabelOperation::Add,
        };
        assert_eq!(describe(&label, "owner/repo"), "add label \"bug\" to issue #9 on owner/repo");

        let comment = GithubAction::Comment {
            target: GithubTarget::PullRequest,
            number: 341,
            body: "looks good".into(),
        };
        assert_eq!(describe(&comment, "owner/repo"), "post a comment on PR #341 on owner/repo");

        let close = GithubAction::SetState {
            target: GithubTarget::Issue,
            number: 9,
            operation: StateOperation::Close,
        };
        assert_eq!(describe(&close, "owner/repo"), "close issue #9 on owner/repo");

        let reopen = GithubAction::SetState {
            target: GithubTarget::PullRequest,
            number: 341,
            operation: StateOperation::Reopen,
        };
        assert_eq!(describe(&reopen, "owner/repo"), "reopen PR #341 on owner/repo");

        let ready = GithubAction::Ready { number: 341 };
        assert_eq!(describe(&ready, "owner/repo"), "mark PR #341 ready for review on owner/repo");
    }
}
