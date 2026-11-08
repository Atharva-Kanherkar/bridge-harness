//! Notices about a row's file that the importer could not apply: a
//! frontmatter value its property cannot read. One per page, kind and
//! property, kept in `page_notices` beside the index. They are not part of the
//! change log, so `rebuild_index` clears them and the next import of a file
//! that still does not read records them again.

use bridge_protocol::messages as wire;
use rusqlite::{params, Connection};

use crate::BridgeError;

pub const UNREADABLE_VALUE: &str = "unreadableValue";

/// Record (or replace) the notice that `property` of `page` did not read.
pub fn record(db: &Connection, space: &str, page: &str, property: &str, raw: &str, expected: &str, seq: i64) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO page_notices(space_id, page_id, kind, property, raw, expected, seq, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(space_id, page_id, kind, property) DO UPDATE SET raw=excluded.raw, expected=excluded.expected, seq=excluded.seq, created_at=excluded.created_at",
        params![space, page, UNREADABLE_VALUE, property, raw.trim(), expected, seq, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Clear the notices of `properties` on `page`.
pub fn clear(db: &Connection, space: &str, page: &str, properties: &[String]) -> Result<(), BridgeError> {
    for property in properties {
        db.execute("DELETE FROM page_notices WHERE space_id=?1 AND page_id=?2 AND property=?3", params![space, page, property])?;
    }
    Ok(())
}

/// Clear every notice of `page` except those of `keep`.
pub fn clear_except(db: &Connection, space: &str, page: &str, keep: &[String]) -> Result<(), BridgeError> {
    let mut statement = db.prepare("SELECT property FROM page_notices WHERE space_id=?1 AND page_id=?2")?;
    let held: Vec<String> = statement.query_map(params![space, page], |row| row.get(0))?.collect::<Result<_, _>>()?;
    let gone: Vec<String> = held.into_iter().filter(|property| !keep.contains(property)).collect();
    clear(db, space, page, &gone)
}

/// Every notice of a page, whatever its space.
pub fn clear_page(db: &Connection, page: &str) -> Result<(), BridgeError> {
    db.execute("DELETE FROM page_notices WHERE page_id=?1", params![page])?;
    Ok(())
}

/// A page's notices, newest first.
pub fn read(db: &Connection, space: &str, page: &str) -> Result<Vec<wire::NoteNotice>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT kind, property, raw, expected, seq, created_at FROM page_notices
         WHERE space_id=?1 AND page_id=?2 ORDER BY created_at DESC, property",
    )?;
    let notices = statement
        .query_map(params![space, page], |row| {
            Ok(wire::NoteNotice { kind: row.get(0)?, property_id: row.get(1)?, raw: row.get(2)?, expected: row.get(3)?, seq: row.get(4)?, created_at: row.get(5)? })
        })?
        .collect::<Result<_, _>>()?;
    Ok(notices)
}
