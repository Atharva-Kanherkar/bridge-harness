//! Reading another tool's SQLite database without changing it, and only the
//! tables Bridge is allowed to read there (Conductor, T3 Code, Orca, Superset).
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

/// The schema versions of one tool's database Bridge reads, by the number
/// the tool records for its newest migration.
///
/// A version number alone is not the safety condition: most releases add
/// tables Bridge never reads. So a database newer than the newest Bridge was
/// checked against is still read, as long as everything Bridge reads at the
/// newest checked version is still there ([`Allowed::missing`]). The table
/// allowlist keeps any new table, secret or not, out of reach either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions {
    /// The oldest version Bridge reads. Anything older is refused.
    pub oldest: i64,
    /// The newest version Bridge was checked against, with a real schema
    /// fixture.
    pub newest_checked: i64,
}

/// Where a database's version falls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// From the oldest version to the newest checked one.
    Checked,
    /// Newer than any version Bridge was checked against.
    Newer,
}

impl Versions {
    /// Where `version` falls, or why it is not read (`what` names the
    /// database for the person: "T3 Code's database").
    pub fn gate(&self, what: &str, version: i64) -> Result<Gate, String> {
        if version < self.oldest {
            return Err(format!("{what} is older than Bridge knows (version {version})"));
        }
        Ok(if version > self.newest_checked { Gate::Newer } else { Gate::Checked })
    }
}

/// Tables and the columns of each Bridge reads.
pub type Reads = &'static [(&'static str, &'static [&'static str])];

/// Why a database without everything Bridge reads is not read. A newer one
/// says that the tool changed what Bridge reads, and names what is gone.
pub fn missing_reason(what: &str, version: i64, gate: Gate, missing: &[String]) -> String {
    match gate {
        Gate::Checked => format!("{what} is missing what Bridge reads (version {version})"),
        Gate::Newer => {
            const SHOWN: usize = 3;
            let mut named = missing.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
            if missing.len() > SHOWN {
                named.push_str(&format!(" and {} more", missing.len() - SHOWN));
            }
            let named = if named.is_empty() { String::new() } else { format!(" ({named})") };
            format!("{what} is newer than Bridge knows (version {version}) and changed what Bridge reads{named}; update Bridge and try again")
        }
    }
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

    /// `table.column` for each one of `reads` the database lacks, in order;
    /// a missing table is every one of its columns.
    pub fn missing(&self, conn: &Connection, reads: Reads) -> rusqlite::Result<Vec<String>> {
        let mut out = Vec::new();
        for (table, columns) in reads {
            let present = self.columns(conn, table)?;
            out.extend(columns.iter().filter(|column| !present.contains(**column)).map(|column| format!("{table}.{column}")));
        }
        Ok(out)
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

impl Allowed {
    /// The key names of a JSON object column, as a JSON array, for each row,
    /// beside the wanted columns: SQLite lists the keys (`json_each`), so
    /// the values (Superset keeps API keys there) never leave the database.
    pub fn select_json_keys(
        &self,
        conn: &Connection,
        table: &str,
        wanted: &[&str],
        json_column: &str,
        alias: &str,
    ) -> rusqlite::Result<Vec<Row>> {
        let present = self.columns(conn, table)?;
        let chosen: Vec<&str> = wanted.iter().copied().filter(|column| present.contains(*column)).collect();
        if chosen.is_empty() {
            return Ok(Vec::new());
        }
        assert!(!chosen.contains(&alias) && !chosen.contains(&json_column), "the JSON column itself is never selected");
        let mut selected: Vec<String> = chosen.iter().map(|column| format!("\"{column}\"")).collect();
        selected.push(if present.contains(json_column) {
            format!("(SELECT json_group_array(key) FROM json_each(CASE WHEN json_valid(\"{json_column}\") AND json_type(\"{json_column}\") = 'object' THEN \"{json_column}\" ELSE '{{}}' END))")
        } else {
            "'[]'".to_owned()
        });
        let sql = format!("SELECT {} FROM {table}", selected.join(", "));
        log(&sql);
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let names: Vec<&str> = chosen.iter().copied().chain(std::iter::once(alias)).collect();
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

/// The columns each statement in `statements` selected, by table: what a
/// reader really read, for checking a `NEWEST_READS` list against it.
#[cfg(test)]
pub(crate) fn columns_read(statements: &[String]) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut out: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = std::collections::BTreeMap::new();
    for statement in statements.iter().filter(|statement| !statement.contains("pragma_table_info") && !statement.starts_with("PRAGMA")) {
        let table = statement
            .split(" FROM ")
            .skip(1)
            .filter_map(|rest| rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next())
            .filter(|name| !name.is_empty() && *name != "json_each")
            .last()
            .unwrap_or_else(|| panic!("no table in {statement}"));
        let columns = out.entry(table.to_owned()).or_default();
        for (index, part) in statement.split('"').enumerate() {
            if index % 2 == 1 {
                columns.insert(part.to_owned());
            }
        }
    }
    out
}

/// `reads` is exactly what a reader reads at the newest checked version:
/// every column it selected (`statements`) is listed, every listed column is
/// in the database, and every listed one was used by some statement.
#[cfg(test)]
pub(crate) fn assert_reads_match(allowed: &Allowed, conn: &Connection, reads: Reads, statements: &[String]) {
    let listed: HashMap<&str, &[&str]> = reads.iter().copied().collect();
    assert_eq!(listed.len(), reads.len(), "a table is listed twice");
    for (table, columns) in columns_read(statements) {
        let listed = listed.get(table.as_str()).unwrap_or_else(|| panic!("{table} is read but not listed"));
        for column in columns {
            assert!(listed.contains(&column.as_str()), "{table}.{column} is read but not listed");
        }
    }
    assert_eq!(allowed.missing(conn, reads).unwrap(), Vec::<String>::new(), "the newest checked schema has every listed column");
    for (table, columns) in reads {
        for column in *columns {
            let used = statements.iter().any(|statement| {
                let words: HashSet<&str> = statement.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
                words.contains(table) && words.contains(column)
            });
            assert!(used, "{table}.{column} is listed but no statement uses it");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSIONS: Versions = Versions { oldest: 5, newest_checked: 102 };
    const ALLOWED: Allowed = Allowed { tool: "Test", tables: &["things", "gone"] };

    #[test]
    fn the_gate_refuses_older_reads_checked_and_marks_newer_at_each_edge() {
        let older = VERSIONS.gate("Test's database", 4).unwrap_err();
        assert_eq!(older, "Test's database is older than Bridge knows (version 4)");
        assert_eq!(VERSIONS.gate("x", i64::MIN).unwrap_err(), format!("x is older than Bridge knows (version {})", i64::MIN));
        assert_eq!(VERSIONS.gate("x", 5), Ok(Gate::Checked));
        assert_eq!(VERSIONS.gate("x", 50), Ok(Gate::Checked));
        assert_eq!(VERSIONS.gate("x", 102), Ok(Gate::Checked));
        assert_eq!(VERSIONS.gate("x", 103), Ok(Gate::Newer));
        assert_eq!(VERSIONS.gate("x", i64::MAX), Ok(Gate::Newer));
        let one = Versions { oldest: 7, newest_checked: 7 };
        assert_eq!((one.gate("x", 6).is_err(), one.gate("x", 7), one.gate("x", 8)), (true, Ok(Gate::Checked), Ok(Gate::Newer)));
    }

    #[test]
    fn missing_lists_absent_columns_and_every_column_of_an_absent_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE things(id TEXT, name TEXT);").unwrap();
        let reads: Reads = &[("things", &["id", "name", "kind"]), ("gone", &["a", "b"])];
        assert_eq!(ALLOWED.missing(&conn, reads).unwrap(), ["things.kind", "gone.a", "gone.b"]);
        assert!(ALLOWED.missing(&conn, &[("things", &["name", "id"])]).unwrap().is_empty());
        assert!(ALLOWED.missing(&conn, &[]).unwrap().is_empty());
    }

    #[test]
    fn columns_read_names_the_table_and_quoted_columns_of_each_kind_of_select() {
        let statements = [
            "SELECT name FROM pragma_table_info('things')".to_owned(),
            r#"SELECT "id", "name" FROM things WHERE id = ?1 ORDER BY name"#.to_owned(),
            r#"SELECT "id", CASE WHEN json_valid("doc") THEN json_extract("doc", '$.a') END FROM gone"#.to_owned(),
            r#"SELECT "id", (SELECT json_group_array(key) FROM json_each(CASE WHEN json_valid("env") THEN "env" ELSE '{}' END)) FROM gone"#.to_owned(),
        ];
        let read = columns_read(&statements);
        assert_eq!(read.keys().collect::<Vec<_>>(), ["gone", "things"]);
        assert_eq!(read["things"].iter().collect::<Vec<_>>(), ["id", "name"]);
        assert_eq!(read["gone"].iter().collect::<Vec<_>>(), ["doc", "env", "id"]);
    }

    #[test]
    #[should_panic(expected = "not on the allowed list")]
    fn missing_never_looks_at_a_table_off_the_allowed_list() {
        let conn = Connection::open_in_memory().unwrap();
        let _ = ALLOWED.missing(&conn, &[("secrets", &["token"])]);
    }

    #[test]
    fn the_reason_says_a_newer_tool_changed_what_bridge_reads_and_names_at_most_three() {
        let names = |count: usize| (0..count).map(|index| format!("t.c{index}")).collect::<Vec<_>>();
        assert_eq!(missing_reason("X", 9, Gate::Checked, &names(2)), "X is missing what Bridge reads (version 9)");
        assert_eq!(
            missing_reason("X", 120, Gate::Newer, &names(1)),
            "X is newer than Bridge knows (version 120) and changed what Bridge reads (t.c0); update Bridge and try again"
        );
        assert_eq!(
            missing_reason("X", 120, Gate::Newer, &names(3)),
            "X is newer than Bridge knows (version 120) and changed what Bridge reads (t.c0, t.c1, t.c2); update Bridge and try again"
        );
        assert_eq!(
            missing_reason("X", 120, Gate::Newer, &names(5)),
            "X is newer than Bridge knows (version 120) and changed what Bridge reads (t.c0, t.c1, t.c2 and 2 more); update Bridge and try again"
        );
        assert_eq!(
            missing_reason("X", 120, Gate::Newer, &[]),
            "X is newer than Bridge knows (version 120) and changed what Bridge reads; update Bridge and try again"
        );
    }
}
