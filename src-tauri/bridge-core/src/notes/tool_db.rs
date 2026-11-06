//! Databases as agents see them: by name. Agents name databases, properties,
//! options and related rows; the wire names them by id. This module maps one
//! to the other, and an unknown name fails with the names that would work,
//! before anything is written.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use super::database::{self, DatabaseDoc};
use super::doc::{PropertyDef, PropertyType, SortDef};
use super::links;
use super::query::{QuerySpec, Row};
use super::service::{NewOption, NewProperty, NotesService};
use super::tool::ToolError;

/// The system properties `where` and `sort` may name, as they are spelled.
pub const SYSTEM_NAMES: [&str; 10] = ["title", "kind", "state", "tags", "project", "chats", "parent", "created", "updated", "createdBy"];

fn bad(message: impl Into<String>) -> ToolError {
    ToolError { status: 400, message: message.into() }
}

fn service(error: crate::BridgeError) -> ToolError {
    bad(error.to_string())
}

fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The person's databases, by title, for an error message.
fn database_titles(notes: &NotesService) -> Result<String, ToolError> {
    let titles: Vec<String> = notes
        .with_db(|db| {
            let mut statement = db.prepare(
                "SELECT COALESCE(title, 'Untitled') FROM pages WHERE kind='database' AND deleted_at IS NULL AND space_id=?1
                 AND id NOT IN (SELECT system_database_id FROM spaces WHERE system_database_id IS NOT NULL) ORDER BY title",
            )?;
            let titles = statement.query_map(params![notes.space()], |row| row.get(0))?.collect::<Result<_, _>>()?;
            Ok(titles)
        })
        .map_err(service)?;
    Ok(if titles.is_empty() { "none yet".to_owned() } else { titles.join(", ") })
}

/// A database of this space named by id or by title (resolved like
/// `[[title]]`).
pub fn find_database(notes: &NotesService, reference: &str) -> Result<DatabaseDoc, ToolError> {
    let reference = reference.trim();
    let system = notes.system_database().ok();
    let found: Option<String> = notes
        .with_db(|db| {
            Ok(db
                .query_row(
                    "SELECT id FROM pages WHERE kind='database' AND deleted_at IS NULL AND space_id=?1 AND (id=?2 OR link_key=?3)
                     ORDER BY id<>?2, created_at, id LIMIT 1",
                    params![notes.space(), reference, links::link_key(reference)],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .map_err(service)?;
    match found.filter(|id| Some(id) != system.as_ref()) {
        Some(id) => notes.database(&id).map_err(service),
        None => Err(bad(format!("no database {reference}; the databases are {}", database_titles(notes)?))),
    }
}

fn property_names(database: &DatabaseDoc) -> String {
    let mut names: Vec<String> = database.ordered().into_iter().map(|(_, def)| def.name.clone()).collect();
    names.extend(SYSTEM_NAMES.iter().map(|name| (*name).to_owned()));
    names.join(", ")
}

/// A property named by an agent: a schema property's id, or a system id.
fn property_id<'a>(database: Option<&'a DatabaseDoc>, name: &str) -> Result<(String, Option<&'a PropertyDef>), ToolError> {
    if let Some(database) = database {
        if let Some((id, def)) = database.schema.iter().find(|(_, def)| fold(&def.name) == fold(name)) {
            return Ok((id.clone(), Some(def)));
        }
    }
    if let Some(system) = SYSTEM_NAMES.iter().find(|system| fold(system) == fold(name)) {
        return Ok(((*system).to_owned(), None));
    }
    Err(bad(match database {
        Some(database) => format!("no property {name}; this database has {}", property_names(database)),
        None => format!("no property {name}; without a database, filter by {}", SYSTEM_NAMES.join(", ")),
    }))
}

fn option_names(def: &PropertyDef) -> String {
    if def.options.is_empty() {
        "no options yet".to_owned()
    } else {
        def.options.iter().map(|option| option.name.clone()).collect::<Vec<_>>().join(", ")
    }
}

/// An option named by an agent, or its id.
fn option_id(def: &PropertyDef, raw: &Value) -> Result<String, ToolError> {
    let name = raw.as_str().ok_or_else(|| bad(format!("{} takes an option name", def.name)))?;
    if let Some(option) = def.option(name).or_else(|| def.option_by_name(name)) {
        return Ok(option.id.clone());
    }
    Err(bad(format!("{} has no option {name}; choose one of {}", def.name, option_names(def))))
}

/// A related row named by title or id, inside the property's target database.
fn related_id(notes: &NotesService, def: &PropertyDef, raw: &Value) -> Result<String, ToolError> {
    let reference = raw.as_str().ok_or_else(|| bad(format!("{} takes page titles", def.name)))?.trim().trim_start_matches("[[").trim_end_matches("]]").to_owned();
    let target = def.relation.as_ref().map(|relation| relation.database_id.clone()).unwrap_or_default();
    let found: Option<String> = notes
        .with_db(|db| {
            Ok(db
                .query_row(
                    "SELECT id FROM pages WHERE parent_id=?1 AND deleted_at IS NULL AND (id=?2 OR link_key=?3) ORDER BY id<>?2, created_at, id LIMIT 1",
                    params![target, reference, links::link_key(&reference)],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .map_err(service)?;
    found.ok_or_else(|| bad(format!("{} names no row {reference} of its database", def.name)))
}

fn map_list(value: Option<&Value>, def: &PropertyDef, each: impl Fn(&Value) -> Result<String, ToolError>) -> Result<Value, ToolError> {
    let items = value.and_then(Value::as_array).ok_or_else(|| bad(format!("{} takes a list", def.name)))?;
    Ok(Value::Array(items.iter().map(|item| each(item).map(Value::String)).collect::<Result<_, _>>()?))
}

/// A `where` tree with names, as the filter tree the planner takes.
pub fn filter(notes: &NotesService, database: Option<&DatabaseDoc>, tree: &Value) -> Result<Value, ToolError> {
    let object = tree.as_object().ok_or_else(|| bad("where is {and: [...]}, {or: [...]} or {property, op, value}"))?;
    for key in ["and", "or"] {
        if let Some(items) = object.get(key) {
            let items = items.as_array().ok_or_else(|| bad(format!("{key} takes a list")))?;
            let mapped: Vec<Value> = items.iter().map(|item| filter(notes, database, item)).collect::<Result<_, _>>()?;
            return Ok(json!({ key: mapped }));
        }
    }
    let name = object.get("property").and_then(Value::as_str).ok_or_else(|| bad("a condition needs a property"))?;
    let op = object.get("op").and_then(Value::as_str).ok_or_else(|| bad("a condition needs an op"))?;
    let (id, def) = property_id(database, name)?;
    let value = object.get("value");
    let mapped = match (def, op) {
        (Some(def), "is" | "isNot" | "contains" | "notContains") if def.kind.options() => Some(Value::String(option_id(def, value.unwrap_or(&Value::Null))?)),
        (Some(def), "isAnyOf" | "isNoneOf" | "containsAny") if def.kind.options() => Some(map_list(value, def, |item| option_id(def, item))?),
        (Some(def), "contains" | "notContains") if def.kind == PropertyType::Relation => Some(Value::String(related_id(notes, def, value.unwrap_or(&Value::Null))?)),
        (Some(def), "containsAny") if def.kind == PropertyType::Relation => Some(map_list(value, def, |item| related_id(notes, def, item))?),
        _ => value.cloned(),
    };
    let mut condition = json!({"property": id, "op": op});
    if let Some(value) = mapped {
        condition["value"] = value;
    }
    Ok(condition)
}

/// `sort` with names, as the planner's sorts.
pub fn sorts(database: Option<&DatabaseDoc>, sort: &Value) -> Result<Vec<SortDef>, ToolError> {
    let items = sort.as_array().ok_or_else(|| bad("sort is a list of {property, direction}"))?;
    items
        .iter()
        .map(|item| {
            let name = item.get("property").and_then(Value::as_str).ok_or_else(|| bad("a sort needs a property"))?;
            let direction = item.get("direction").and_then(Value::as_str).unwrap_or("asc").to_owned();
            Ok(SortDef { property: property_id(database, name)?.0, direction })
        })
        .collect()
}

/// `properties` by name, as the cells `set_values` takes, each checked
/// against its property so a refused one writes nothing.
pub fn cells(notes: &NotesService, database: &DatabaseDoc, properties: &Value) -> Result<Vec<(String, Option<Value>)>, ToolError> {
    let object = properties.as_object().ok_or_else(|| bad("properties is an object of property name to value"))?;
    let mut cells = Vec::new();
    for (name, value) in object {
        let Some((id, def)) = database.schema.iter().find(|(_, def)| fold(&def.name) == fold(name)) else {
            let names: Vec<String> = database.ordered().into_iter().map(|(_, def)| def.name.clone()).collect();
            return Err(bad(format!("no property {name}; this database has {}", if names.is_empty() { "no properties".into() } else { names.join(", ") })));
        };
        if value.is_null() {
            cells.push((id.clone(), None));
            continue;
        }
        let converted = match def.kind {
            PropertyType::Select | PropertyType::Status => Value::String(option_id(def, value)?),
            PropertyType::MultiSelect => map_list(Some(value), def, |item| option_id(def, item))?,
            PropertyType::Relation => map_list(Some(value), def, |item| related_id(notes, def, item))?,
            PropertyType::Date => match value {
                Value::String(day) => json!({"start": day}),
                other => other.clone(),
            },
            _ => value.clone(),
        };
        database::validate_value(def, &converted).map_err(service)?;
        cells.push((id.clone(), Some(converted)));
    }
    Ok(cells)
}

/// A row's cells by property name, options and related rows by name.
pub fn named_cells(notes: &NotesService, database: &DatabaseDoc, values: &BTreeMap<String, Value>) -> Value {
    let title_of = |id: &str| -> String {
        notes
            .with_db(|db| Ok(db.query_row("SELECT title FROM pages WHERE id=?1", params![id], |row| row.get::<_, Option<String>>(0)).optional()?.flatten()))
            .ok()
            .flatten()
            .unwrap_or_else(|| id.to_owned())
    };
    let mut named = Map::new();
    for (id, def) in database.ordered() {
        let Some(value) = values.get(id) else { continue };
        let option = |raw: &Value| raw.as_str().and_then(|id| def.option(id)).map(|option| Value::String(option.name.clone())).unwrap_or(Value::Null);
        let shown = match def.kind {
            PropertyType::Select | PropertyType::Status => option(value),
            PropertyType::MultiSelect => Value::Array(value.as_array().map(|items| items.iter().map(option).collect()).unwrap_or_default()),
            PropertyType::Relation => Value::Array(value.as_array().map(|items| items.iter().filter_map(Value::as_str).map(|id| Value::String(title_of(id))).collect()).unwrap_or_default()),
            _ => value.clone(),
        };
        named.insert(def.name.clone(), shown);
    }
    Value::Object(named)
}

/// A database's schema as an agent reads it.
pub fn schema(database: &DatabaseDoc) -> Value {
    let mut views: Vec<(&String, &super::doc::ViewDef)> = database.views.iter().collect();
    views.sort_by_key(|(id, view)| (view.order, id.as_str()));
    json!({
        "properties": database.ordered().into_iter().map(|(_, def)| {
            let mut property = json!({"name": def.name, "type": def.kind.as_str()});
            if def.kind.options() {
                property["options"] = json!(def.options.iter().map(|option| option.name.clone()).collect::<Vec<_>>());
            }
            property
        }).collect::<Vec<_>>(),
        "views": views.into_iter().map(|(_, view)| view.name.clone()).collect::<Vec<_>>(),
    })
}

fn cell_text(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(cell_text).collect::<Vec<_>>().join(", "),
        Value::Object(object) => match (object.get("start"), object.get("end")) {
            (Some(start), Some(end)) => format!("{} – {}", cell_text(start), cell_text(end)),
            (Some(start), None) => cell_text(start),
            _ => value.to_string(),
        },
        other => other.to_string(),
    };
    text.replace('|', "\\|").replace('\n', " ")
}

/// The first rows of a database as a Markdown table, titles first.
pub fn table(notes: &NotesService, database: &DatabaseDoc, limit: usize) -> Result<(String, usize), ToolError> {
    let result = notes.query_database(&database.id, &QuerySpec { limit: Some(limit), ..Default::default() }).map_err(service)?;
    let columns: Vec<String> = database.ordered().into_iter().map(|(_, def)| def.name.clone()).collect();
    let mut out = format!("| Title | {} |\n|{}\n", columns.join(" | "), "---|".repeat(columns.len() + 1));
    for Row { note, values } in &result.rows {
        let named = named_cells(notes, database, values);
        let cells: Vec<String> = columns.iter().map(|column| named.get(column).map(cell_text).unwrap_or_default()).collect();
        out.push_str(&format!("| {} | {} |\n", cell_text(&json!(note.title.clone().unwrap_or_else(|| "Untitled".into()))), cells.join(" | ")));
    }
    Ok((out, result.total))
}

/// `asDatabase.properties` as the properties a new database starts with.
pub fn new_properties(notes: &NotesService, spec: &Value) -> Result<Vec<NewProperty>, ToolError> {
    let items = spec.get("properties").and_then(Value::as_array).ok_or_else(|| bad("asDatabase needs properties: [{name, type, options?}]"))?;
    items
        .iter()
        .map(|item| {
            let name = item.get("name").and_then(Value::as_str).ok_or_else(|| bad("a property needs a name"))?.to_owned();
            let kind = item.get("type").and_then(Value::as_str).ok_or_else(|| bad(format!("{name} needs a type: one of {}", database::PROPERTY_TYPES.join(", "))))?.to_owned();
            let options = item
                .get("options")
                .and_then(Value::as_array)
                .map(|options| options.iter().filter_map(Value::as_str).map(|option| NewOption { name: option.to_owned(), ..Default::default() }).collect())
                .unwrap_or_default();
            let relation = match item.get("database").and_then(Value::as_str) {
                Some(reference) => Some(find_database(notes, reference)?.id),
                None => None,
            };
            Ok(NewProperty { name, kind, options, relation, date_format: None })
        })
        .collect()
}
