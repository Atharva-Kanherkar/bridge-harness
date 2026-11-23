//! Reading another tool's SQLite database without changing it, and only the
//! tables Bridge is allowed to read there (Conductor, T3 Code, Orca).
//!
//! Every query names its table and columns. A table that is not on the
//! caller's allowed list is a bug, not a runtime branch: it panics, and the
//! deny-list tests run every reader against the real schemas.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
thread_local! {
    /// Every statement a reader ran on this thread, for the deny-list tests.
    pub(crate) static STATEMENTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn log(sql: &str) {
    #[cfg(test)]
    STATEMENTS.with(|statements| statements.borrow_mut().push(sql.to_owned()));
    #[cfg(not(test))]
    let _ = sql;
}

/// Open a database without writing anything next to it.
///
/// These are WAL databases. While the tool runs, its `-wal` and `-shm` files
/// hold changes not yet in the main file, so Bridge opens it read-only and
/// reads through them. Once the tool quits they are gone, and even a
/// read-only open would create them; so with no live WAL the file is opened
/// immutable, which reads it as it is and creates nothing.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let wal = PathBuf::from(format!("{}-wal", path.to_string_lossy()));
    let live = std::fs::metadata(&wal).is_ok_and(|metadata| metadata.len() > 0);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let uri = |query: &str| format!("file:{}?{query}", uri_path(path));
    let conn = if live {
        Connection::open_with_flags(uri("mode=ro"), flags).or_else(|_| Connection::open_with_flags(uri("mode=ro&immutable=1"), flags))?
    } else {
        Connection::open_with_flags(uri("mode=ro&immutable=1"), flags)?
    };
    conn.busy_timeout(Duration::from_secs(3))?;
    Ok(conn)
}

/// `PRAGMA user_version`: the schema version of a tool that keeps no
/// migration table (Orca).
pub fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    let sql = "PRAGMA user_version";
    log(sql);
    conn.query_row(sql, [], |row| row.get(0))
}

/// A path as a `file:` URI path: everything but unreserved characters and
/// `/` percent-encoded (`Application Support` has a space).
pub fn uri_path(path: &Path) -> String {
    path.to_string_lossy()
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The tables one tool's reader may query.
#[derive(Debug, Clone, Copy)]
pub struct Allowed {
    pub tool: &'static str,
    pub tables: &'static [&'static str],
}

impl Allowed {
    fn check(&self, table: &str) {
        assert!(self.tables.contains(&table), "{} table {table} is not on the allowed list", self.tool);
    }

    /// A table's columns; empty when the table does not exist.
    pub fn columns(&self, conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
        self.check(table);
        let sql = format!("SELECT name FROM pragma_table_info('{table}')");
        log(&sql);
        let mut statement = conn.prepare(&sql)?;
        let names = statement.query_map([], |row| row.get::<_, String>(0))?;
        names.collect()
    }

    /// Rows of a table, as the wanted columns that exist; a missing column
    /// reads as absent.
    pub fn select(&self, conn: &Connection, table: &str, wanted: &[&str]) -> rusqlite::Result<Vec<Row>> {
        self.select_where(conn, table, wanted, None, &[])
    }

    /// The same, filtered by `filter` (a `WHERE` clause over named columns,
    /// with `?` parameters).
    pub fn select_where(
        &self,
        conn: &Connection,
        table: &str,
        wanted: &[&str],
        filter: Option<&str>,
        params: &[&dyn ToSql],
    ) -> rusqlite::Result<Vec<Row>> {
        let present = self.columns(conn, table)?;
        let chosen: Vec<&str> = wanted.iter().copied().filter(|column| present.contains(*column)).collect();
        if chosen.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = format!("SELECT {} FROM {table}", chosen.iter().map(|column| format!("\"{column}\"")).collect::<Vec<_>>().join(", "));
        if let Some(filter) = filter {
            sql.push_str(" WHERE ");
            sql.push_str(filter);
        }
        log(&sql);
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query(params)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut cells = HashMap::new();
            for (index, column) in chosen.iter().enumerate() {
                cells.insert((*column).to_owned(), Cell::of(row.get_ref(index)?));
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// Rows of a table as the wanted columns plus named fields of one JSON
    /// column, extracted by SQLite. The rest of that JSON never leaves the
    /// database, for columns that also hold things Bridge must not read
    /// (Orca keeps a worker's environment beside its session id). A field
    /// is `(name, path)`, with `path` a fixed `$.…` JSON path; invalid JSON
    /// reads as absent. No row comes back when the JSON column is missing.
    pub fn select_json(
        &self,
        conn: &Connection,
        table: &str,
        wanted: &[&str],
        json_column: &str,
        fields: &[(&str, &str)],
        filter: Option<&str>,
        params: &[&dyn ToSql],
    ) -> rusqlite::Result<Vec<Row>> {
        let present = self.columns(conn, table)?;
        if !present.contains(json_column) {
            return Ok(Vec::new());
        }
        let chosen: Vec<&str> = wanted.iter().copied().filter(|column| present.contains(*column)).collect();
        let mut selected: Vec<String> = chosen.iter().map(|column| format!("\"{column}\"")).collect();
        for (name, path) in fields {
            assert!(path.starts_with('$') && !path.contains('\'') && !chosen.contains(name), "bad JSON field {name} {path}");
            selected.push(format!("CASE WHEN json_valid(\"{json_column}\") THEN json_extract(\"{json_column}\", '{path}') END"));
        }
        let mut sql = format!("SELECT {} FROM {table}", selected.join(", "));
        if let Some(filter) = filter {
            sql.push_str(" WHERE ");
            sql.push_str(filter);
        }
        log(&sql);
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query(params)?;
        let names: Vec<&str> = chosen.iter().copied().chain(fields.iter().map(|(name, _)| *name)).collect();
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut cells = HashMap::new();
            for (index, name) in names.iter().enumerate() {
                cells.insert((*name).to_owned(), Cell::of(row.get_ref(index)?));
            }
            out.push(cells);
        }
        Ok(out)
    }
}

/// One row: column name to value.
pub type Row = HashMap<String, Cell>;

/// One value, read loosely: these tools' columns are mostly untyped.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Text(String),
}

impl Cell {
    pub fn of(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Integer(number) => Self::Int(number),
            ValueRef::Real(number) => Self::Text(number.to_string()),
            ValueRef::Text(bytes) => Self::Text(String::from_utf8_lossy(bytes).into_owned()),
            ValueRef::Null | ValueRef::Blob(_) => Self::Null,
        }
    }

    pub fn text(&self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text.clone()).filter(|text| !text.trim().is_empty()),
            Self::Int(number) => Some(number.to_string()),
            Self::Null => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Self::Int(number) => Some(*number),
            Self::Text(text) => text.trim().parse().ok(),
            Self::Null => None,
        }
    }
}

/// A column as text: absent, null and blank are all `None`.
pub fn text(row: &Row, column: &str) -> Option<String> {
    row.get(column).and_then(Cell::text)
}

pub fn int(row: &Row, column: &str) -> Option<i64> {
    row.get(column).and_then(Cell::int)
}
