//! The trail rows: a trail, its members, and the records a person took off it.
//!
//! Members are references, not copies. One record sits on at most one trail
//! (`PRIMARY KEY(kind, ref)`), so "which trail is this chat on" has one answer.
//! A dismissal remembers a record the person removed, so the automatic sweep
//! never puts it back.

use bridge_protocol::messages::{TrailAddedBy, TrailMember, TrailMemberKind, TrailMemberRef, MAX_TRAIL_TITLE_CHARS};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::BridgeError;

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS trails (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            workspace_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            archived_at TEXT
        );
        CREATE TABLE IF NOT EXISTS trail_members (
            trail_id TEXT NOT NULL REFERENCES trails(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            ref TEXT NOT NULL,
            added_by TEXT NOT NULL,
            added_at TEXT NOT NULL,
            PRIMARY KEY (kind, ref)
        );
        CREATE INDEX IF NOT EXISTS idx_trail_members_trail ON trail_members(trail_id, added_at);
        CREATE TABLE IF NOT EXISTS trail_dismissals (
            trail_id TEXT NOT NULL REFERENCES trails(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            ref TEXT NOT NULL,
            dismissed_at TEXT NOT NULL,
            PRIMARY KEY (trail_id, kind, ref)
        );",
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailRow {
    pub id: String,
    pub title: String,
    pub workspace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

const COLUMNS: &str = "id,title,workspace_id,created_at,updated_at,archived_at";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrailRow> {
    Ok(TrailRow {
        id: row.get(0)?,
        title: row.get(1)?,
        workspace_id: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        archived_at: row.get(5)?,
    })
}

/// The title as stored: trimmed, inner whitespace runs kept, 1 to 120 chars.
pub fn clean_title(title: &str) -> Result<String, BridgeError> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(BridgeError::Invalid("A trail needs a title.".into()));
    }
    if title.chars().count() > MAX_TRAIL_TITLE_CHARS {
        return Err(BridgeError::Invalid(format!("A trail title is at most {MAX_TRAIL_TITLE_CHARS} characters.")));
    }
    Ok(title)
}

pub fn create(db: &Connection, title: &str, workspace_id: Option<&str>, now: &str) -> Result<TrailRow, BridgeError> {
    let title = clean_title(title)?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO trails(id,title,workspace_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
        params![id, title, workspace_id, now],
    )?;
    require(db, &id)
}

pub fn get(db: &Connection, id: &str) -> Result<Option<TrailRow>, BridgeError> {
    Ok(db.query_row(&format!("SELECT {COLUMNS} FROM trails WHERE id=?1"), params![id], row).optional()?)
}

pub fn require(db: &Connection, id: &str) -> Result<TrailRow, BridgeError> {
    get(db, id)?.ok_or_else(|| BridgeError::Invalid("That trail no longer exists.".into()))
}

pub fn list(db: &Connection, include_archived: bool) -> Result<Vec<TrailRow>, BridgeError> {
    let filter = if include_archived { "" } else { " WHERE archived_at IS NULL" };
    let mut statement = db.prepare(&format!("SELECT {COLUMNS} FROM trails{filter} ORDER BY updated_at DESC, id"))?;
    let rows = statement.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn touch(db: &Connection, id: &str, now: &str) -> Result<(), BridgeError> {
    db.execute("UPDATE trails SET updated_at=?2 WHERE id=?1", params![id, now])?;
    Ok(())
}

pub fn rename(db: &Connection, id: &str, title: &str, now: &str) -> Result<TrailRow, BridgeError> {
    let title = clean_title(title)?;
    require(db, id)?;
    db.execute("UPDATE trails SET title=?2, updated_at=?3 WHERE id=?1", params![id, title, now])?;
    require(db, id)
}

/// Archive or unarchive. Members stay; nothing a member points at changes.
pub fn set_archived(db: &Connection, id: &str, archived: bool, now: &str) -> Result<TrailRow, BridgeError> {
    require(db, id)?;
    if archived {
        db.execute("UPDATE trails SET archived_at=COALESCE(archived_at, ?2), updated_at=?2 WHERE id=?1", params![id, now])?;
    } else {
        db.execute("UPDATE trails SET archived_at=NULL, updated_at=?2 WHERE id=?1", params![id, now])?;
    }
    require(db, id)
}

pub fn members(db: &Connection, trail_id: &str) -> Result<Vec<TrailMember>, BridgeError> {
    let mut statement =
        db.prepare("SELECT kind,ref,added_by,added_at FROM trail_members WHERE trail_id=?1 ORDER BY added_at, kind, ref")?;
    let rows = statement
        .query_map(params![trail_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(kind, reference, added_by, added_at)| {
            Some(TrailMember {
                kind: TrailMemberKind::parse(&kind)?,
                reference,
                added_by: if added_by == "auto" { TrailAddedBy::Auto } else { TrailAddedBy::Person },
                added_at,
            })
        })
        .collect())
}

/// The trail a record sits on, if any.
pub fn trail_of(db: &Connection, member: &TrailMemberRef) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT trail_id FROM trail_members WHERE kind=?1 AND ref=?2",
            params![member.kind.as_str(), member.reference],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn is_dismissed(db: &Connection, trail_id: &str, member: &TrailMemberRef) -> Result<bool, BridgeError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM trail_dismissals WHERE trail_id=?1 AND kind=?2 AND ref=?3)",
        params![trail_id, member.kind.as_str(), member.reference],
        |row| row.get::<_, i64>(0),
    )? == 1)
}

/// Put a record on a trail. `Ok(false)` when it is already there. A record on
/// another trail is refused with that trail's title. A person adding a record
/// clears its dismissal; the sweep never adds a dismissed one (it checks first).
pub fn add_member(
    db: &Connection,
    trail_id: &str,
    member: &TrailMemberRef,
    added_by: TrailAddedBy,
    now: &str,
) -> Result<bool, BridgeError> {
    if member.reference.trim().is_empty() {
        return Err(BridgeError::Invalid("A trail member needs a reference.".into()));
    }
    require(db, trail_id)?;
    match trail_of(db, member)? {
        Some(existing) if existing == trail_id => return Ok(false),
        Some(existing) => {
            let title = get(db, &existing)?.map(|trail| trail.title).unwrap_or_default();
            return Err(BridgeError::Invalid(format!("This is already on the trail “{title}”. Remove it there first.")));
        }
        None => {}
    }
    let by = match added_by {
        TrailAddedBy::Person => "person",
        TrailAddedBy::Auto => "auto",
    };
    db.execute(
        "INSERT INTO trail_members(trail_id,kind,ref,added_by,added_at) VALUES(?1,?2,?3,?4,?5)",
        params![trail_id, member.kind.as_str(), member.reference, by, now],
    )?;
    if added_by == TrailAddedBy::Person {
        db.execute(
            "DELETE FROM trail_dismissals WHERE trail_id=?1 AND kind=?2 AND ref=?3",
            params![trail_id, member.kind.as_str(), member.reference],
        )?;
    }
    touch(db, trail_id, now)?;
    Ok(true)
}

/// Take a record off a trail and remember that it was taken off.
pub fn remove_member(db: &Connection, trail_id: &str, member: &TrailMemberRef, now: &str) -> Result<bool, BridgeError> {
    require(db, trail_id)?;
    let removed = db.execute(
        "DELETE FROM trail_members WHERE trail_id=?1 AND kind=?2 AND ref=?3",
        params![trail_id, member.kind.as_str(), member.reference],
    )?;
    db.execute(
        "INSERT OR REPLACE INTO trail_dismissals(trail_id,kind,ref,dismissed_at) VALUES(?1,?2,?3,?4)",
        params![trail_id, member.kind.as_str(), member.reference, now],
    )?;
    if removed > 0 {
        touch(db, trail_id, now)?;
    }
    Ok(removed > 0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::store;
    use std::path::Path;

    pub fn database() -> Connection {
        store::open(Path::new(":memory:")).unwrap()
    }

    pub fn session(id: &str) -> TrailMemberRef {
        TrailMemberRef { kind: TrailMemberKind::Session, reference: id.into() }
    }

    #[test]
    fn create_refuses_blank_titles_and_trims() {
        let db = database();
        assert!(create(&db, "   ", None, "t0").is_err());
        assert!(create(&db, &"x".repeat(121), None, "t0").is_err());
        let trail = create(&db, "  Archive   a chat ", Some("w"), "t0").unwrap();
        assert_eq!(trail.title, "Archive a chat");
        assert_eq!(trail.workspace_id.as_deref(), Some("w"));
        assert_eq!(rename(&db, &trail.id, " Renamed ", "t1").unwrap().title, "Renamed");
        assert!(rename(&db, &trail.id, "", "t1").is_err());
    }

    #[test]
    fn a_record_sits_on_one_trail() {
        let db = database();
        let first = create(&db, "First", None, "t0").unwrap();
        let second = create(&db, "Second", None, "t0").unwrap();
        assert!(add_member(&db, &first.id, &session("s1"), TrailAddedBy::Person, "t1").unwrap());
        let refused = add_member(&db, &second.id, &session("s1"), TrailAddedBy::Person, "t1").unwrap_err();
        assert!(refused.to_string().contains("“First”"), "{refused}");
        assert_eq!(trail_of(&db, &session("s1")).unwrap().as_deref(), Some(first.id.as_str()));
    }

    #[test]
    fn adding_twice_is_a_no_op() {
        let db = database();
        let trail = create(&db, "T", None, "t0").unwrap();
        assert!(add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t1").unwrap());
        assert!(!add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Auto, "t2").unwrap());
        let members = members(&db, &trail.id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].added_by, TrailAddedBy::Person);
        assert_eq!(members[0].added_at, "t1");
    }

    #[test]
    fn removing_records_a_dismissal_and_manual_add_clears_it() {
        let db = database();
        let trail = create(&db, "T", None, "t0").unwrap();
        add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Auto, "t1").unwrap();
        assert!(remove_member(&db, &trail.id, &session("s1"), "t2").unwrap());
        assert!(is_dismissed(&db, &trail.id, &session("s1")).unwrap());
        assert!(members(&db, &trail.id).unwrap().is_empty());
        add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t3").unwrap();
        assert!(!is_dismissed(&db, &trail.id, &session("s1")).unwrap());
    }

    #[test]
    fn archive_keeps_members_and_touches_no_source_rows() {
        let db = database();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,depth,title) VALUES('s1',NULL,'codex','Chat','idle','estimated','direct',0,'Fix login')",
            [],
        )
        .unwrap();
        let before: String = db.query_row("SELECT json_array(id,harness,label,status,title) FROM sessions WHERE id='s1'", [], |r| r.get(0)).unwrap();
        let trail = create(&db, "T", None, "t0").unwrap();
        add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t1").unwrap();
        let archived = set_archived(&db, &trail.id, true, "t2").unwrap();
        assert_eq!(archived.archived_at.as_deref(), Some("t2"));
        assert!(list(&db, false).unwrap().is_empty());
        assert_eq!(list(&db, true).unwrap().len(), 1);
        assert_eq!(members(&db, &trail.id).unwrap().len(), 1);
        assert!(set_archived(&db, &trail.id, false, "t3").unwrap().archived_at.is_none());
        remove_member(&db, &trail.id, &session("s1"), "t4").unwrap();
        let after: String = db.query_row("SELECT json_array(id,harness,label,status,title) FROM sessions WHERE id='s1'", [], |r| r.get(0)).unwrap();
        assert_eq!(before, after);
    }
}
