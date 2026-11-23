//! The review job's transition table. Every status write in `store` checks
//! here first, so an illegal move is an error rather than a silent overwrite.
//!
//! ```text
//! queued -> preparing -> reviewing -> drafted -> posting -> posted
//!             |             |                      |
//!             +-> queued <--+   (lease reclaim)    +-> drafted (post failed, retry)
//! ```
//!
//! `failed`, `cancelled` and `interrupted` are reachable from every
//! non-terminal status. Nothing leaves a terminal status: a retry is a new job.

use bridge_protocol::messages::ReviewJobStatus as Status;

use crate::BridgeError;

/// Whether a job may move from `from` to `to`.
pub fn allowed(from: Status, to: Status) -> bool {
    if from.is_terminal() {
        return false;
    }
    if matches!(to, Status::Failed | Status::Cancelled | Status::Interrupted) {
        return true;
    }
    matches!(
        (from, to),
        (Status::Queued, Status::Preparing)
            | (Status::Preparing, Status::Reviewing)
            | (Status::Reviewing, Status::Drafted)
            | (Status::Drafted, Status::Posting)
            | (Status::Posting, Status::Posted)
            // A runner died holding the lease: the job goes back to the queue.
            | (Status::Preparing, Status::Queued)
            | (Status::Reviewing, Status::Queued)
            // A post a person asked for failed: back to the draft to try again.
            | (Status::Posting, Status::Drafted)
    )
}

/// [`allowed`] as an error a caller can return.
pub fn check(from: Status, to: Status) -> Result<(), BridgeError> {
    if allowed(from, to) {
        Ok(())
    } else {
        Err(BridgeError::Invalid(format!(
            "a review cannot move from {} to {}",
            from.as_str(),
            to.as_str()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGAL: &[(Status, Status)] = &[
        (Status::Queued, Status::Preparing),
        (Status::Preparing, Status::Reviewing),
        (Status::Reviewing, Status::Drafted),
        (Status::Drafted, Status::Posting),
        (Status::Posting, Status::Posted),
        (Status::Preparing, Status::Queued),
        (Status::Reviewing, Status::Queued),
        (Status::Posting, Status::Drafted),
    ];

    fn is_exit(to: Status) -> bool {
        matches!(to, Status::Failed | Status::Cancelled | Status::Interrupted)
    }

    #[test]
    fn every_legal_transition_is_accepted() {
        for &(from, to) in LEGAL {
            assert!(allowed(from, to), "{} -> {}", from.as_str(), to.as_str());
            check(from, to).unwrap();
        }
    }

    #[test]
    fn every_other_transition_is_refused() {
        for from in Status::ALL {
            for to in Status::ALL {
                let expected = !from.is_terminal() && (is_exit(to) || LEGAL.contains(&(from, to)));
                assert_eq!(allowed(from, to), expected, "{} -> {}", from.as_str(), to.as_str());
                assert_eq!(check(from, to).is_ok(), expected);
            }
        }
    }

    #[test]
    fn terminal_statuses_have_no_exits() {
        for from in Status::ALL.into_iter().filter(|status| status.is_terminal()) {
            for to in Status::ALL {
                assert!(!allowed(from, to), "{} -> {}", from.as_str(), to.as_str());
            }
        }
        let refused = check(Status::Posted, Status::Queued).unwrap_err().to_string();
        assert!(refused.contains("posted") && refused.contains("queued"), "{refused}");
    }

    #[test]
    fn failed_cancelled_and_interrupted_are_reachable_from_every_non_terminal_status() {
        for from in Status::ALL.into_iter().filter(|status| !status.is_terminal()) {
            for to in [Status::Failed, Status::Cancelled, Status::Interrupted] {
                assert!(allowed(from, to), "{} -> {}", from.as_str(), to.as_str());
            }
        }
    }

    #[test]
    fn no_shortcut_skips_the_draft_or_the_post() {
        assert!(!allowed(Status::Reviewing, Status::Posting), "auto mode still passes through drafted");
        assert!(!allowed(Status::Reviewing, Status::Posted));
        assert!(!allowed(Status::Queued, Status::Reviewing));
        assert!(!allowed(Status::Drafted, Status::Posted));
        assert!(!allowed(Status::Drafted, Status::Queued), "a draft is never re-reviewed in place");
    }
}
