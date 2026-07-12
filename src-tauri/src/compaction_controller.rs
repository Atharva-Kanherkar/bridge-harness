use crate::{session_forest::EntryKind, session_forest::SessionForest, BridgeError};
use rusqlite::Connection;

/// Issue #9 owns the durable lifecycle hook. Issue #10 extends this boundary
/// with checkpoint prompting, validation, repair, and projection.
pub struct CompactionController;

impl CompactionController {
    pub fn request_before_suspend(
        db: &Connection,
        session_id: &str,
        reason: &str,
    ) -> Result<(), BridgeError> {
        SessionForest::new(db)
            .append(
                session_id,
                EntryKind::CompactionRequested,
                serde_json::json!({"reason": reason}),
            )
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        Ok(())
    }
}
