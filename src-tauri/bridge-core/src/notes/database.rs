//! Databases: a page whose children are its rows.
//!
//! A database page stores its scope, one schema entry per property and one
//! entry per saved view in its own doc. A row stores its values in its own doc,
//! keyed by property id, so renaming a column is one schema edit rather than a
//! rewrite of every row. This module owns the rules around those records:
//! property and value validation, the conversions a retype makes, the name
//! rules, the values index, and the system database's seeded views.
//!
//! The index in `database_props` and `page_values` is rebuilt from the docs on
//! every write (and by `service::rebuild_index`), never written independently.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::doc::{self, OptionDef, PropertyDef, PropertyType, ViewDef, ViewType};
use super::ids::new_id;
use super::service::invalid;
use crate::BridgeError;

/// Every property type a person can add.
pub const PROPERTY_TYPES: [&str; 9] = ["text", "number", "select", "multiSelect", "date", "checkbox", "url", "relation", "status"];
/// The system properties every page already has. Views address them by id and
/// they are never written to a row's values map.
pub const SYSTEM_PROPERTIES: [&str; 9] = ["title", "kind", "state", "tags", "project", "chats", "created", "updated", "createdBy"];
/// The parent page: filterable because the "All" view excludes child tasks,
/// not a property a person can add or set.
pub const FILTER_ONLY_PROPERTIES: [&str; 1] = ["parent"];
pub const STATUS_GROUPS: [&str; 3] = ["todo", "inProgress", "done"];
pub const SCOPES: [&str; 2] = ["children", "space"];

/// A database page's records, read from its doc.
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseDoc {
    pub id: String,
    pub scope: String,
    pub schema: BTreeMap<String, PropertyDef>,
    pub views: BTreeMap<String, ViewDef>,
    /// Old property names, mapping each to the property id that owned it.
    pub retired: BTreeMap<String, String>,
}

impl DatabaseDoc {
    /// Properties in the order a file, a table and a picker show them.
    pub fn ordered(&self) -> Vec<(&String, &PropertyDef)> {
        let mut ordered: Vec<_> = self.schema.iter().collect();
        ordered.sort_by_key(|(id, def)| (def.order, id.as_str()));
        ordered
    }

    /// The property names a view can address, for an error message.
    pub fn property_names(&self) -> String {
        let mut names: Vec<&str> = self.ordered().into_iter().map(|(_, def)| def.name.as_str()).collect();
        names.sort_unstable();
        names.join(", ")
    }
}

/// Read a database from `pages` and its doc, or `None` when `id` is not a
/// live database page.
pub fn read(db: &Connection, id: &str) -> Result<Option<DatabaseDoc>, BridgeError> {
    let kind: Option<String> = db
        .query_row("SELECT kind FROM pages WHERE id=?1 AND deleted_at IS NULL", params![id], |row| row.get(0))
        .optional()?;
    if kind.as_deref() != Some("database") {
        return Ok(None);
    }
    let page = doc::load_page(db, id, None)?.0;
    Ok(Some(DatabaseDoc {
        id: id.to_owned(),
        scope: page.scope().unwrap_or_else(|| "children".to_owned()),
        schema: page.schema(),
        views: page.views(),
        retired: page.retired(),
    }))
}

/// The database a page is a row of, when its parent is one.
pub fn database_of(db: &Connection, page_id: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT p.parent_id FROM pages p
             JOIN pages d ON d.id=p.parent_id AND d.kind='database' AND d.deleted_at IS NULL
             WHERE p.id=?1 AND p.deleted_at IS NULL",
            params![page_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

/// The rows of a database, newest last.
pub fn row_ids(db: &Connection, database_id: &str) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare("SELECT id FROM pages WHERE parent_id=?1 AND deleted_at IS NULL ORDER BY created_at, id")?;
    let ids: Vec<String> = statement.query_map(params![database_id], |row| row.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// One property's value on every row that has one, with the row id.
pub fn row_values(db: &Connection, database_id: &str, prop: &str) -> Result<Vec<(String, Value)>, BridgeError> {
    let mut out = Vec::new();
    for row in row_ids(db, database_id)? {
        if let Some(value) = doc::load_page(db, &row, None)?.0.values().remove(prop) {
            out.push((row, value));
        }
    }
    Ok(out)
}

/// A property name, trimmed, or the reason it cannot be used.
pub fn check_name(schema: &BTreeMap<String, PropertyDef>, name: &str, except: Option<&str>) -> Result<String, BridgeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("a property needs a name"));
    }
    if name.chars().count() > 80 {
        return Err(invalid("a property name is at most 80 characters"));
    }
    // The name becomes a plain YAML key in every row's file.
    let lowered = name.to_lowercase();
    if name.starts_with(['-', '?', ':', '#', '&', '*', '!', '|', '>', '@', '`']) || name.chars().any(|character| matches!(character, ':' | '#' | '[' | ']' | '{' | '}' | ',' | '&' | '*' | '?' | '|' | '>' | '!' | '@' | '`' | '"' | '\'' | '%' | '\n' | '\r' | '\t')) {
        return Err(invalid(format!("{name} cannot be a property name: it needs to be a plain YAML key")));
    }
    // YAML reads these as booleans, null or numbers, not as a key a person can read back.
    if matches!(lowered.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~") || lowered.parse::<f64>().is_ok() {
        return Err(invalid(format!("{name} cannot be a property name: YAML would not read it as one")));
    }
    if SYSTEM_PROPERTIES.iter().any(|id| id.to_lowercase() == lowered) {
        return Err(invalid(format!("{name} is a system property; choose another name")));
    }
    if super::markdown::KNOWN.iter().any(|key| key.to_lowercase() == lowered) {
        return Err(invalid(format!("{name} is reserved in the page's frontmatter; choose another name")));
    }
    for (id, def) in schema {
        if Some(id.as_str()) != except && def.name.to_lowercase() == lowered {
            return Err(invalid(format!("this database already has a property named {name}")));
        }
    }
    Ok(name.to_owned())
}

/// One option name, trimmed and unique within its property.
pub fn check_option_name(def: &PropertyDef, name: &str, except: Option<&str>) -> Result<String, BridgeError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid(format!("{} needs a name for its option", def.name)));
    }
    let lowered = name.to_lowercase();
    if def.options.iter().any(|option| option.name.to_lowercase() == lowered && Some(option.id.as_str()) != except) {
        return Err(invalid(format!("{} already has an option named {name}", def.name)));
    }
    Ok(name.to_owned())
}

/// The value a property of this type accepts, or the reason it does not.
pub fn validate_value(def: &PropertyDef, value: &Value) -> Result<(), BridgeError> {
    let bad = |wanted: &str| invalid(format!("{} is {wanted}", def.name));
    match def.kind {
        PropertyType::Text | PropertyType::Url => value.as_str().map(|_| ()).ok_or_else(|| bad("text")),
        PropertyType::Number => match value.as_f64() {
            Some(number) if number.is_finite() => Ok(()),
            _ => Err(bad("a finite number")),
        },
        PropertyType::Checkbox => value.as_bool().map(|_| ()).ok_or_else(|| bad("true or false")),
        PropertyType::Select | PropertyType::Status => {
            let id = value.as_str().ok_or_else(|| bad("an option"))?;
            if def.option(id).is_some() {
                Ok(())
            } else {
                Err(invalid(format!("{} has no option {id}; choose one of {}", def.name, option_names(def))))
            }
        }
        PropertyType::MultiSelect => {
            let items = value.as_array().ok_or_else(|| bad("a list of options"))?;
            let mut seen = std::collections::BTreeSet::new();
            for item in items {
                let id = item.as_str().ok_or_else(|| bad("a list of options"))?;
                if def.option(id).is_none() {
                    return Err(invalid(format!("{} has no option {id}; choose one of {}", def.name, option_names(def))));
                }
                if !seen.insert(id) {
                    return Err(invalid(format!("{} lists the same option twice", def.name)));
                }
            }
            Ok(())
        }
        PropertyType::Date => {
            let object = value.as_object().ok_or_else(|| invalid(format!("{} is {{start, end?}}, not {}", def.name, value)))?;
            let start = object.get("start").and_then(Value::as_str).ok_or_else(|| bad("a date with a start"))?;
            let Some(start_at) = instant(start) else {
                return Err(invalid(format!("{} starts at a non-date: {start}", def.name)));
            };
            if let Some(end) = object.get("end") {
                let end = end.as_str().ok_or_else(|| bad("a date"))?;
                let Some(end_at) = instant(end) else {
                    return Err(invalid(format!("{} ends at a non-date: {end}", def.name)));
                };
                if end_at < start_at {
                    return Err(invalid(format!("{} ends before it starts", def.name)));
                }
            }
            if object.keys().any(|key| !matches!(key.as_str(), "start" | "end")) {
                return Err(bad("a date with only start and end"));
            }
            Ok(())
        }
        PropertyType::Relation => {
            let items = value.as_array().ok_or_else(|| bad("a list of page ids"))?;
            let mut seen = std::collections::BTreeSet::new();
            for item in items {
                let id = item.as_str().filter(|id| !id.trim().is_empty()).ok_or_else(|| bad("a list of page ids"))?;
                if !seen.insert(id) {
                    return Err(invalid(format!("{} lists the same page twice", def.name)));
                }
            }
            Ok(())
        }
    }
}

fn option_names(def: &PropertyDef) -> String {
    if def.options.is_empty() {
        "no options yet".to_owned()
    } else {
        let mut names: Vec<&str> = def.options.iter().map(|option| option.name.as_str()).collect();
        names.sort_unstable();
        names.join(", ")
    }
}

/// `YYYY-MM-DD` or RFC 3339.
pub fn check_date(raw: &str) -> bool {
    instant(raw).is_some()
}

/// A date or a datetime as one comparable instant. A bare day is midnight UTC.
pub fn instant(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = raw.trim();
    if raw.len() == 10 {
        let day = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
        return Some(chrono::DateTime::from_naive_utc_and_offset(day.and_hms_opt(0, 0, 0)?, chrono::Utc));
    }
    Some(chrono::DateTime::parse_from_rfc3339(raw).ok()?.with_timezone(&chrono::Utc))
}

/// The `date_v` an index row stores. A `datetime` keeps its instant in UTC; a
/// `date` keeps its day.
fn date_key(def: &PropertyDef, raw: &str) -> Option<String> {
    let at = instant(raw)?;
    if def.date_format.as_deref() == Some("datetime") {
        Some(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    } else {
        Some(at.date_naive().format("%Y-%m-%d").to_string())
    }
}

/// The value a retype produces. `None` clears the cell. A retype to `select`
/// or `status` may add options: the ids have to exist before values that name
/// them are written.
pub fn convert(old: &PropertyDef, next: &mut PropertyDef, value: &Value) -> Option<Value> {
    use PropertyType::*;
    match (old.kind, next.kind) {
        (Number, Text) => Some(Value::String(number_text(value)?)),
        (Text, Number) => value.as_str()?.trim().parse::<f64>().ok().filter(|number| number.is_finite()).map(|number| json!(number)),
        (Select, Text) | (Status, Text) => Some(Value::String(old.option(value.as_str()?)?.name.clone())),
        (Text, Select) | (Text, Status) => {
            let name = value.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            if let Some(option) = next.option_by_name(name) {
                return Some(Value::String(option.id.clone()));
            }
            let option = OptionDef {
                id: new_id(),
                name: name.to_owned(),
                color: None,
                group: (next.kind == Status).then(|| "todo".to_owned()),
            };
            let id = option.id.clone();
            next.options.push(option);
            Some(Value::String(id))
        }
        (Select, MultiSelect) => Some(json!([value.as_str()?])),
        (Checkbox, Text) => Some(Value::String(if value.as_bool()? { "true".into() } else { "false".into() })),
        (Text, Checkbox) => match value.as_str()?.trim().to_lowercase().as_str() {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        _ => None,
    }
}

fn number_text(value: &Value) -> Option<String> {
    let number = value.as_f64()?;
    Some(if number.fract() == 0.0 && number.abs() < 1e15 { format!("{}", number as i64) } else { number.to_string() })
}

// ---------------------------------------------------------------------------
// Index maintenance
// ---------------------------------------------------------------------------

/// Rebuild one page's rows in `database_props` and `page_values` from the doc
/// the write path already holds.
pub(crate) fn reindex_page(db: &Connection, space: &str, page_id: &str, live: &doc::PageDoc) -> Result<(), BridgeError> {
    db.execute("DELETE FROM page_values WHERE space_id=?1 AND page_id=?2", params![space, page_id])?;
    db.execute("DELETE FROM database_props WHERE space_id=?1 AND database_id=?2", params![space, page_id])?;
    let props = live.props();
    if props.deleted_at.is_some() {
        // A tombstoned database leaves no values behind for the rows it owned.
        db.execute("DELETE FROM page_values WHERE space_id=?1 AND database_id=?2", params![space, page_id])?;
        return Ok(());
    }
    if props.kind.as_deref() == Some("database") {
        // The rows keep their values; only this database's own records change.
        return write_props(db, space, page_id, &live.schema());
    }
    let Some(parent) = props.parent_id else { return Ok(()) };
    let Some(database) = read(db, &parent)? else { return Ok(()) };
    write_values(db, space, &parent, page_id, &database.schema, &live.values())
}

/// Rebuild every row of one database from its docs. The database's own space
/// is read here so a caller cannot index it under another one.
pub(crate) fn reindex_database(db: &Connection, database_id: &str) -> Result<(), BridgeError> {
    let space: String = db
        .query_row("SELECT space_id FROM pages WHERE id=?1", params![database_id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| invalid(format!("no page with id {database_id}")))?;
    db.execute("DELETE FROM database_props WHERE space_id=?1 AND database_id=?2", params![space, database_id])?;
    db.execute("DELETE FROM page_values WHERE space_id=?1 AND database_id=?2", params![space, database_id])?;
    let page = doc::load_page(db, database_id, None)?.0;
    let schema = page.schema();
    write_props(db, &space, database_id, &schema)?;
    for row in row_ids(db, database_id)? {
        let values = doc::load_page(db, &row, None)?.0.values();
        write_values(db, &space, database_id, &row, &schema, &values)?;
    }
    Ok(())
}

fn write_props(db: &Connection, space: &str, database_id: &str, schema: &BTreeMap<String, PropertyDef>) -> Result<(), BridgeError> {
    let mut statement = db.prepare(
        "INSERT OR REPLACE INTO database_props(space_id, database_id, prop_id, name, type, def) VALUES(?1,?2,?3,?4,?5,?6)",
    )?;
    for (prop_id, def) in schema {
        statement.execute(params![space, database_id, prop_id, def.name, def.kind.as_str(), serde_json::to_string(def).unwrap_or_default()])?;
    }
    Ok(())
}

/// One index row of a value: a scalar, or one item of a multi-select or a
/// relation.
struct ScalarRow {
    item: i64,
    text: Option<String>,
    number: Option<f64>,
    date: Option<String>,
}

fn write_values(
    db: &Connection,
    space: &str,
    database_id: &str,
    page_id: &str,
    schema: &BTreeMap<String, PropertyDef>,
    values: &BTreeMap<String, Value>,
) -> Result<(), BridgeError> {
    let mut statement = db.prepare(
        "INSERT OR REPLACE INTO page_values(space_id, database_id, page_id, prop_id, item, text_v, num_v, date_v)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
    )?;
    for (prop_id, value) in values {
        // A value whose property was deleted stays in the doc but is not indexed.
        let Some(def) = schema.get(prop_id) else { continue };
        for row in scalar_rows(def, value) {
            statement.execute(params![space, database_id, page_id, prop_id, row.item, row.text, row.number, row.date])?;
        }
    }
    Ok(())
}

/// One row per scalar, `text_v` for everything a text comparison can match.
fn scalar_rows(def: &PropertyDef, value: &Value) -> Vec<ScalarRow> {
    let row = |item: i64, text: Option<String>, number: Option<f64>, date: Option<String>| ScalarRow { item, text, number, date };
    match def.kind {
        PropertyType::Number => value.as_f64().map(|number| vec![row(0, None, Some(number), None)]).unwrap_or_default(),
        PropertyType::Date => {
            let key = value.get("start").and_then(Value::as_str).and_then(|start| date_key(def, start));
            key.map(|key| vec![row(0, None, None, Some(key))]).unwrap_or_default()
        }
        PropertyType::MultiSelect | PropertyType::Relation => value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(item, value)| value.as_str().map(|text| row(item as i64, Some(text.to_owned()), None, None)))
                    .collect()
            })
            .unwrap_or_default(),
        PropertyType::Checkbox => value.as_bool().map(|flag| vec![row(0, Some(flag.to_string()), None, None)]).unwrap_or_default(),
        _ => value.as_str().map(|text| vec![row(0, Some(text.to_owned()), None, None)]).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

/// A value as the text a person reads in their editor.
pub fn render_value(db: &Connection, space: &str, def: &PropertyDef, value: &Value) -> Option<String> {
    match def.kind {
        PropertyType::Text | PropertyType::Url => value.as_str().map(super::markdown::scalar),
        PropertyType::Number => {
            let number = value.as_f64()?;
            Some(number_text(&json!(number))?)
        }
        PropertyType::Checkbox => value.as_bool().map(|flag| flag.to_string()),
        PropertyType::Select | PropertyType::Status => {
            let name = &def.option(value.as_str()?)?.name;
            Some(super::markdown::scalar(name))
        }
        PropertyType::MultiSelect => {
            let names: Vec<&str> = value.as_array()?.iter().filter_map(|item| def.option(item.as_str()?).map(|option| option.name.as_str())).collect();
            serde_json::to_string(&names).ok()
        }
        PropertyType::Date => {
            let start = value.get("start")?.as_str()?;
            match value.get("end").and_then(Value::as_str) {
                Some(end) => serde_json::to_string(&json!({ "start": start, "end": end })).ok(),
                None => Some(super::markdown::scalar(start)),
            }
        }
        PropertyType::Relation => {
            let titles: Vec<String> = value.as_array()?.iter().filter_map(Value::as_str).map(|id| relation_target(db, space, def, id)).collect();
            serde_json::to_string(&titles).ok()
        }
    }
}

/// A related page as `[[Title]]`, or its id when the title is missing or
/// ambiguous, so the file always reads back to the same row.
fn relation_target(db: &Connection, space: &str, def: &PropertyDef, id: &str) -> String {
    let title: Option<String> = db
        .query_row("SELECT title FROM pages WHERE id=?1 AND space_id=?2 AND deleted_at IS NULL", params![id, space], |row| row.get(0))
        .optional()
        .ok()
        .flatten()
        .flatten();
    let Some(title) = title else { return id.to_owned() };
    let scoped = def.relation.as_ref().map(|relation| relation.database_id.as_str());
    let matches: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE space_id=?1 AND link_key=?2 AND deleted_at IS NULL AND (?3 IS NULL OR parent_id=?3)",
            params![space, super::links::link_key(&title), scoped],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if matches == 1 {
        format!("[[{title}]]")
    } else {
        id.to_owned()
    }
}

/// The row a relation item names: a page id, or a `[[Title]]` resolved inside
/// the database the property points at.
fn relation_page(db: &Connection, space: &str, def: &PropertyDef, raw: &str) -> Option<String> {
    let target = raw.trim();
    let by_id: Option<String> = db
        .query_row("SELECT id FROM pages WHERE id=?1 AND space_id=?2 AND deleted_at IS NULL", params![target, space], |row| row.get(0))
        .optional()
        .ok()
        .flatten();
    if by_id.is_some() {
        return by_id;
    }
    let title = target.trim_start_matches("[[").trim_end_matches("]]");
    let key = super::links::link_key(title);
    let database = def.relation.as_ref().map(|relation| relation.database_id.clone());
    db.query_row(
        "SELECT id FROM pages WHERE space_id=?1 AND link_key=?2 AND deleted_at IS NULL AND (?3 IS NULL OR parent_id=?3) ORDER BY created_at, id LIMIT 1",
        params![space, key, database],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// A frontmatter value parsed by its property's type, or `None` when it does
/// not parse (the line is then kept verbatim as an unknown key).
pub fn parse_value(db: &Connection, space: &str, def: &PropertyDef, raw: &str) -> Option<Value> {
    match def.kind {
        PropertyType::Text | PropertyType::Url => super::markdown::read_scalar(raw).map(Value::String),
        PropertyType::Number => {
            let number: Value = serde_json::from_str(raw.trim()).ok()?;
            if !number.is_number() || !number.as_f64().is_some_and(f64::is_finite) {
                return None;
            }
            Some(number)
        }
        PropertyType::Checkbox => match raw.trim().to_lowercase().as_str() {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        PropertyType::Select | PropertyType::Status => {
            let name = super::markdown::read_scalar(raw)?;
            Some(Value::String(def.option_by_name(&name)?.id.clone()))
        }
        PropertyType::MultiSelect => {
            let names = super::markdown::read_list(raw);
            let ids: Option<Vec<Value>> = names.into_iter().map(|name| Some(Value::String(def.option_by_name(&name)?.id.clone()))).collect();
            Some(Value::Array(ids?))
        }
        PropertyType::Date => {
            let raw = raw.trim();
            let value = if raw.starts_with('{') {
                serde_json::from_str::<Value>(raw).ok()?
            } else {
                json!({ "start": super::markdown::read_scalar(raw)? })
            };
            validate_value(def, &value).ok()?;
            Some(value)
        }
        PropertyType::Relation => {
            let items = super::markdown::read_list(raw);
            let ids: Option<Vec<Value>> = items.into_iter().map(|item| Some(Value::String(relation_page(db, space, def, &item)?))).collect();
            Some(Value::Array(ids?))
        }
    }
}

// ---------------------------------------------------------------------------
// The system database
// ---------------------------------------------------------------------------

/// The filter operators a saved view may use. #888 owns the executor and its
/// per-type rules; this table is what the seeded views are written against and
/// what their test validates, so the two cannot drift silently.
pub const FILTER_OPS: [&str; 6] = ["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty"];

/// The views every space starts with, reproducing the Notes list's tabs.
///
/// A view executor hides the system database itself the same way
/// `notes/list_notes` does, from `spaces.system_database_id`: a saved filter
/// cannot say "not the list's own database".
pub fn default_views() -> Vec<(String, ViewDef)> {
    let view = |name: &str, order: i64, filter: Value| ViewDef {
        name: name.to_owned(),
        kind: ViewType::List,
        order,
        filter: Some(filter),
        sorts: Vec::new(),
        group_by: None,
        date_property: None,
        properties: Vec::new(),
    };
    let condition = |property: &str, op: &str, value: Option<Value>| {
        let mut condition = json!({ "property": property, "op": op });
        if let Some(value) = value {
            condition["value"] = value;
        }
        condition
    };
    let all = |item: Value| json!({ "and": [item] });
    let and = |items: Vec<Value>| json!({ "and": items });
    let or = |items: Vec<Value>| json!({ "or": items });
    let not_archived = || condition("state", "isNot", Some(json!("archived")));
    vec![
        (
            new_id(),
            view(
                "All",
                0,
                and(vec![
                    not_archived(),
                    or(vec![condition("kind", "isNot", Some(json!("task"))), condition("parent", "isEmpty", None)]),
                ]),
            ),
        ),
        (
            new_id(),
            view(
                "Inbox",
                1,
                and(vec![
                    condition("state", "is", Some(json!("inbox"))),
                    // Today's Inbox never shows a task that sits under a page.
                    or(vec![condition("kind", "isNot", Some(json!("task"))), condition("parent", "isEmpty", None)]),
                ]),
            ),
        ),
        (
            new_id(),
            view("Pages", 2, and(vec![condition("kind", "isNot", Some(json!("task"))), not_archived()])),
        ),
        (
            new_id(),
            view("Tasks", 3, and(vec![condition("kind", "is", Some(json!("task"))), not_archived()])),
        ),
        (new_id(), view("Ideas", 4, all(condition("kind", "is", Some(json!("idea")))))),
        (
            new_id(),
            view("In progress with an agent", 5, and(vec![condition("state", "is", Some(json!("doing"))), condition("chats", "isNotEmpty", None)])),
        ),
        (new_id(), view("Archived", 6, all(condition("state", "is", Some(json!("archived")))))),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::notes::service::tests::service;
    use crate::notes::service::{CreatePage, NotesService, PERSON};

    pub(crate) fn database(notes: &NotesService, title: &str) -> String {
        notes.create(CreatePage { title: Some(title.into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id
    }

    pub(crate) fn property(notes: &NotesService, id: &str, name: &str, kind: PropertyType) -> String {
        notes.update_schema(id, &[super::super::service::SchemaOp::Add(super::super::service::NewProperty { name: name.into(), kind: kind.as_str().into(), ..Default::default() })], PERSON).unwrap();
        notes.database(id).unwrap().ordered().into_iter().find(|(_, def)| def.name == name).map(|(id, _)| id.clone()).unwrap()
    }

    /// `count` rows with a title and a rating, for the query performance gate.
    pub(crate) fn seed_rows(notes: &NotesService, id: &str, count: usize) {
        let rating = property(notes, id, "Rating", PropertyType::Number);
        for index in 0..count {
            let row = notes
                .create(CreatePage { title: Some(format!("Book {index:04}")), parent_id: Some(id.to_owned()), ..Default::default() }, PERSON)
                .unwrap();
            notes.set_values(&row.id, &[(rating.clone(), Some(json!((index % 5) as f64 + 1.0)))], PERSON).unwrap();
        }
    }

    #[test]
    fn value_validation_covers_every_type() {
        let status = PropertyDef {
            name: "Status".into(),
            kind: PropertyType::Status,
            order: 0,
            options: vec![
                OptionDef { id: "todo".into(), name: "To do".into(), color: None, group: Some("todo".into()) },
                OptionDef { id: "done".into(), name: "Done".into(), color: None, group: Some("done".into()) },
            ],
            relation: None,
            date_format: None,
        };
        assert!(validate_value(&status, &json!("todo")).is_ok());
        for bad in [json!("elsewhere"), json!(1), json!(null), json!(["todo"])] {
            assert!(validate_value(&status, &bad).is_err(), "{bad}");
        }
        let error = validate_value(&status, &json!("elsewhere")).unwrap_err().to_string();
        assert!(error.contains("To do") && error.contains("Done"), "{error}");
        for (kind, good, bad) in [
            (PropertyType::Text, json!("words"), json!(3)),
            (PropertyType::Url, json!("https://bridge.dev"), json!(false)),
            (PropertyType::Number, json!(4.5), json!("4.5")),
            (PropertyType::Checkbox, json!(true), json!("true")),
            (PropertyType::Date, json!({"start": "2026-11-10"}), json!("2026-11-10")),
            (PropertyType::Date, json!({"start": "2026-11-10", "end": "2026-11-12"}), json!({"start": "2026-11-12", "end": "2026-11-01"})),
            (PropertyType::Relation, json!(["page-1"]), json!([" "])),
            (PropertyType::MultiSelect, json!(["todo"]), json!(["todo", "done", "elsewhere"])),
            (PropertyType::MultiSelect, json!(["todo"]), json!(["todo", "todo"])),
            (PropertyType::Relation, json!(["page-1"]), json!(["page-1", "page-1"])),
        ] {
            let def = PropertyDef { name: "F".into(), kind, order: 0, options: status.options.clone(), relation: None, date_format: None };
            assert!(validate_value(&def, &good).is_ok(), "{kind:?} refused {good}");
            assert!(validate_value(&def, &bad).is_err(), "{kind:?} accepted {bad}");
        }
    }

    #[test]
    fn a_retype_converts_what_it_can_and_clears_the_rest() {
        let option = OptionDef { id: "o1".into(), name: "Done".into(), color: None, group: None };
        let select = PropertyDef { name: "S".into(), kind: PropertyType::Select, order: 0, options: vec![option.clone()], relation: None, date_format: None };
        let mut to_multi = PropertyDef { kind: PropertyType::MultiSelect, ..select.clone() };
        assert_eq!(convert(&select, &mut to_multi, &json!("o1")), Some(json!(["o1"])));

        let mut to_text = PropertyDef { kind: PropertyType::Text, ..select.clone() };
        assert_eq!(convert(&select, &mut to_text, &json!("o1")), Some(json!("Done")));

        let text = PropertyDef { name: "S".into(), kind: PropertyType::Text, order: 0, options: vec![], relation: None, date_format: None };
        let mut to_select = PropertyDef { kind: PropertyType::Select, options: vec![option.clone()], ..text.clone() };
        assert_eq!(convert(&text, &mut to_select, &json!("Done")), Some(json!("o1")), "an existing option is matched by name");
        let created = convert(&text, &mut to_select, &json!("Blocked")).unwrap();
        assert_eq!(to_select.options.len(), 2);
        assert_eq!(to_select.option(created.as_str().unwrap()).unwrap().name, "Blocked");

        let number = PropertyDef { kind: PropertyType::Number, ..text.clone() };
        let mut from_number = PropertyDef { kind: PropertyType::Text, ..text.clone() };
        assert_eq!(convert(&number, &mut from_number, &json!(5)), Some(json!("5")));
        let mut to_number = PropertyDef { kind: PropertyType::Number, ..text.clone() };
        assert_eq!(convert(&text, &mut to_number, &json!("5")), Some(json!(5.0)));
        assert_eq!(convert(&text, &mut to_number, &json!("five")), None);

        let checkbox = PropertyDef { kind: PropertyType::Checkbox, ..text.clone() };
        let mut to_flag = PropertyDef { kind: PropertyType::Checkbox, ..text.clone() };
        assert_eq!(convert(&text, &mut to_flag, &json!("true")), Some(json!(true)));
        assert_eq!(convert(&text, &mut to_flag, &json!("yes")), None);
        let mut from_flag = PropertyDef { kind: PropertyType::Text, ..text.clone() };
        assert_eq!(convert(&checkbox, &mut from_flag, &json!(false)), Some(json!("false")));

        let date = PropertyDef { kind: PropertyType::Date, ..text.clone() };
        let mut to_date = PropertyDef { kind: PropertyType::Number, ..text.clone() };
        assert_eq!(convert(&date, &mut to_date, &json!("2026-11-10")), None, "anything else clears");
    }

    #[test]
    fn property_names_are_unique_and_never_shadow_system_or_frontmatter_keys() {
        let mut schema = BTreeMap::new();
        schema.insert("p1".to_owned(), PropertyDef { name: "Rating".into(), kind: PropertyType::Number, order: 0, options: vec![], relation: None, date_format: None });
        assert_eq!(check_name(&schema, "  Due  ", None).unwrap(), "Due");
        assert_eq!(check_name(&schema, "rating", Some("p1")).unwrap(), "rating", "its own name is not a duplicate");
        for bad in ["rating", "Title", "state", "createdBy", "id", "parent", "tags", "database", "", "- Due", "true", "null", "2026"] {
            assert!(check_name(&schema, bad, None).is_err(), "{bad} was accepted");
        }
        assert!(check_name(&schema, "Été", None).is_ok());
        schema.insert("p2".to_owned(), PropertyDef { name: "Été".into(), kind: PropertyType::Text, order: 1, options: vec![], relation: None, date_format: None });
        assert!(check_name(&schema, "été", None).is_err(), "one Unicode fold for every comparison");
        schema.remove("p2");
        assert!(check_name(&schema, "Stars & Stripes", None).is_err(), "a name has to be a plain YAML key");
    }

    #[test]
    fn a_datetime_indexes_its_instant_and_a_date_its_day() {
        let date = PropertyDef { name: "Due".into(), kind: PropertyType::Date, order: 0, options: vec![], relation: None, date_format: Some("date".into()) };
        let datetime = PropertyDef { date_format: Some("datetime".into()), ..date.clone() };
        let evening = json!({"start": "2026-11-10T23:30:00-08:00"});
        assert_eq!(scalar_rows(&date, &evening)[0].date.as_deref(), Some("2026-11-11"), "a date property indexes the UTC day of the instant");
        assert_eq!(scalar_rows(&datetime, &evening)[0].date.as_deref(), Some("2026-11-11T07:30:00.000Z"), "a datetime keeps its instant");
    }

    #[test]
    fn a_relation_writes_a_title_or_an_id_and_reads_both_back() {
        let (_dir, notes) = service();
        let library = database(&notes, "Library");
        let book = |title: &str| notes.create(CreatePage { title: Some(title.into()), parent_id: Some(library.clone()), ..Default::default() }, PERSON).unwrap();
        let only = book("Unique");
        let first = book("Book");
        let second = book("Book");
        let def = PropertyDef { name: "Related".into(), kind: PropertyType::Relation, order: 0, options: vec![], relation: Some(doc::RelationDef { database_id: library.clone() }), date_format: None };
        let space = notes.space().to_owned();
        let (rendered, parsed) = notes
            .with_db(|db| {
                Ok((
                    render_value(db, &space, &def, &json!([only.id.clone()])),
                    parse_value(db, &space, &def, "[\"[[Unique]]\"]"),
                ))
            })
            .unwrap();
        assert_eq!(rendered.as_deref(), Some("[\"[[Unique]]\"]"));
        assert_eq!(parsed, Some(json!([only.id.clone()])));
        // Two rows share a title, so the file names the row it means.
        let (rendered, parsed) = notes
            .with_db(|db| Ok((render_value(db, &space, &def, &json!([second.id.clone()])), parse_value(db, &space, &def, &format!("[\"{}\"]", second.id)))))
            .unwrap();
        assert_eq!(rendered, Some(format!("[\"{}\"]", second.id)));
        assert_eq!(parsed, Some(json!([second.id])));
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn a_database_indexes_its_properties_and_its_rows_values() {
        let (_dir, notes) = service();
        let id = database(&notes, "Reading list");
        let rating = property(&notes, &id, "Rating", PropertyType::Number);
        let status = property(&notes, &id, "Status", PropertyType::Status);
        let notes_props = |id: &str| -> Vec<(String, String)> {
            notes.with_db(|db| Ok(db.prepare("SELECT name, type FROM database_props WHERE database_id=?1 ORDER BY name")?.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)).unwrap()
        };
        assert_eq!(notes_props(&id), vec![("Rating".to_owned(), "number".to_owned()), ("Status".to_owned(), "status".to_owned())]);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(json!(5)))], PERSON).unwrap();
        let values = |row: &str| -> Vec<(String, i64, Option<String>, Option<f64>, Option<String>)> {
            notes.with_db(|db| Ok(db.prepare("SELECT prop_id, item, text_v, num_v, date_v FROM page_values WHERE page_id=?1 ORDER BY prop_id, item")?.query_map(params![row], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<_, _>>()?)).unwrap()
        };
        let before = values(&row.id);
        assert_eq!(before, vec![(rating.clone(), 0, None, Some(5.0), None)]);
        // Clearing, reparenting and a deleted property all leave the index clean.
        notes.set_values(&row.id, &[(rating.clone(), None)], PERSON).unwrap();
        assert!(values(&row.id).is_empty());
        notes.set_values(&row.id, &[(rating.clone(), Some(json!(4)))], PERSON).unwrap();
        let elsewhere = notes.capture("no database", PERSON).unwrap();
        notes.update(crate::notes::service::UpdateNote { id: row.id.clone(), parent_id: Some(elsewhere.id.clone()), ..Default::default() }, PERSON).unwrap();
        assert!(values(&row.id).is_empty(), "a moved row keeps its values but is not indexed");
        notes.update(crate::notes::service::UpdateNote { id: row.id.clone(), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(values(&row.id).len(), 1, "moving it back brings the value back");
        notes.update_schema(&id, &[crate::notes::service::SchemaOp::Delete { prop: rating.clone() }], PERSON).unwrap();
        assert!(values(&row.id).is_empty());
        assert_eq!(notes.read(&row.id).unwrap().note.kind, "note");
        let _ = status;
    }

    #[test]
    fn the_values_index_rebuilds_from_page_updates() {
        let (_dir, notes) = service();
        let id = database(&notes, "Reading list");
        let rating = property(&notes, &id, "Rating", PropertyType::Number);
        let status = property(&notes, &id, "Status", PropertyType::Status);
        assert!(notes.database(&id).unwrap().schema[&status].options.is_empty(), "a new status property has no options");
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(json!(4)))], PERSON).unwrap();
        let indexed = |notes: &NotesService| -> Vec<(String, Option<f64>)> {
            notes
                .with_db(|db| {
                    let mut statement = db.prepare("SELECT prop_id, num_v FROM page_values WHERE page_id=?1 ORDER BY prop_id")?;
                    let rows: Vec<(String, Option<f64>)> = statement.query_map(params![row.id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
                    Ok(rows)
                })
                .unwrap()
        };
        let before = indexed(&notes);
        assert_eq!(before, vec![(rating.clone(), Some(4.0))]);
        notes.with_db(|db| { db.execute("DELETE FROM database_props", [])?; db.execute("DELETE FROM page_values", [])?; Ok(()) }).unwrap();
        notes.rebuild_index().unwrap();
        assert_eq!(indexed(&notes), before, "the values index comes back from the log");
        let props: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM database_props WHERE database_id=?1", params![id], |r| r.get(0))?)).unwrap();
        assert_eq!(props, 2);
        assert_eq!(notes.database(&id).unwrap().schema.len(), 2);
    }

    #[test]
    fn every_space_has_one_hidden_system_database_with_the_seeded_views() {
        let (_dir, notes) = service();
        let id = notes.system_database().unwrap();
        let list = notes.list(&crate::notes::service::ListFilter::default()).unwrap();
        assert!(list.iter().all(|note| note.id != id), "the system database is not listed");
        let database = notes.database(&id).unwrap();
        assert_eq!(database.scope, "space");
        let names: Vec<&str> = database.views.values().map(|view| view.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec!["All", "Archived", "Ideas", "In progress with an agent", "Inbox", "Pages", "Tasks"]);
        assert_eq!(notes.system_database().unwrap(), id, "creating it is idempotent");
        let reopened = NotesService::open(notes.root().parent().unwrap(), std::sync::Arc::new(|_: &str| true)).unwrap();
        assert_eq!(reopened.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages WHERE kind='database'", [], |r| r.get::<_, i64>(0))?)).unwrap(), 1);
    }

    /// A reference evaluator for the seeded filters. #888's executor is the
    /// real one; this is here so the seeded views can be checked against
    /// today's Notes list on a fixture instead of against their own text.
    fn matches(filter: &Value, note: &bridge_protocol::messages::Note) -> bool {
        if let Some(items) = filter.get("and").and_then(Value::as_array) {
            return items.iter().all(|item| matches(item, note));
        }
        if let Some(items) = filter.get("or").and_then(Value::as_array) {
            return items.iter().any(|item| matches(item, note));
        }
        let property = filter.get("property").and_then(Value::as_str).unwrap_or_default();
        let op = filter.get("op").and_then(Value::as_str).unwrap_or_default();
        assert!(FILTER_OPS.contains(&op), "a seeded view uses an operator the grammar does not have: {op}");
        let wanted = filter.get("value").and_then(Value::as_str);
        let actual: Option<String> = match property {
            "state" => Some(note.state.clone()),
            "kind" => Some(note.kind.clone()),
            "parent" => note.parent_id.clone(),
            "chats" => (!note.session_ids.is_empty()).then(|| "some".to_owned()),
            other => panic!("a seeded view filters on {other}, which the evaluator does not know"),
        };
        match op {
            "is" => actual.as_deref() == wanted,
            "isNot" => actual.as_deref() != wanted,
            "isEmpty" => actual.is_none(),
            "isNotEmpty" => actual.is_some(),
            other => panic!("unhandled op {other}"),
        }
    }

    #[test]
    fn the_seeded_views_reproduce_the_notes_list_tabs() {
        use crate::notes::service::{CreatePage, ListFilter};
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Plan".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Inbox".into()), state: Some("inbox".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Child task".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), state: Some("inbox".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Top task".into()), kind: Some("task".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Old".into()), state: Some("archived".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("An idea".into()), kind: Some("idea".into()), ..Default::default() }, PERSON).unwrap();
        let working = notes.create(CreatePage { title: Some("With an agent".into()), kind: Some("task".into()), state: Some("doing".into()), ..Default::default() }, PERSON).unwrap();
        notes.update(crate::notes::service::UpdateNote { id: working.id, add_session_id: Some("chat".into()), ..Default::default() }, PERSON).unwrap();

        let all = notes.list(&ListFilter { include_archived: true, ..Default::default() }).unwrap();
        // Today's NotesScreen list logic, ported.
        let child_task = |note: &bridge_protocol::messages::Note| note.parent_id.is_some() && note.kind == "task";
        let today = |tab: &str| -> std::collections::BTreeSet<String> {
            all.iter()
                .filter(|note| match tab {
                    "archived" => note.state == "archived",
                    _ if note.state == "archived" => false,
                    "tasks" => note.kind == "task",
                    "pages" => note.kind != "task",
                    _ => !child_task(note) && (tab == "all" || note.state == "inbox"),
                })
                .map(|note| note.id.clone())
                .collect()
        };
        let views = default_views();
        let view = |name: &str| views.iter().find(|(_, view)| view.name == name).unwrap().1.clone();
        for (name, tab) in [("All", "all"), ("Inbox", "inbox"), ("Pages", "pages"), ("Tasks", "tasks"), ("Archived", "archived")] {
            let actual: std::collections::BTreeSet<String> = all
                .iter()
                .filter(|note| matches(view(name).filter.as_ref().unwrap(), note))
                .map(|note| note.id.clone())
                .collect();
            assert_eq!(actual, today(tab), "{name} does not match today's {tab} list");
        }
        let ideas: std::collections::BTreeSet<String> = all.iter().filter(|note| note.kind == "idea").map(|note| note.id.clone()).collect();
        let actual: std::collections::BTreeSet<String> = all.iter().filter(|note| matches(view("Ideas").filter.as_ref().unwrap(), note)).map(|note| note.id.clone()).collect();
        assert_eq!(actual, ideas, "Ideas");
        let doing: std::collections::BTreeSet<String> = all.iter().filter(|note| note.state == "doing" && !note.session_ids.is_empty()).map(|note| note.id.clone()).collect();
        let actual: std::collections::BTreeSet<String> = all
            .iter()
            .filter(|note| matches(view("In progress with an agent").filter.as_ref().unwrap(), note))
            .map(|note| note.id.clone())
            .collect();
        assert_eq!(actual, doing, "In progress with an agent");
        assert_eq!(views.len(), 7);
    }
}
