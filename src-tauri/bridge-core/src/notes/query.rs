//! Database queries: a filter tree, sorts and a grouping compiled to one SQL
//! statement over `pages` and the `page_values` index.
//!
//! Filtering never happens in Rust. Each condition becomes an `IN` (or
//! `NOT IN`) against a `page_values` subquery that the
//! `(space_id, database_id, prop_id, *_v)` indexes answer, or a predicate on a
//! `pages` column for a system property. Every value is a bound parameter.
//! SQL returns the matching ids in sort order with their sort keys; the cursor
//! and the grouping walk that ordered list, so they see exactly what SQL kept.
//!
//! `src/notesMock.ts` (`mockQueryDatabase`) mirrors these rules, and the shared
//! cases in `testing/fixtures/notes-query-cases.json` hold the two together.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use base64::Engine;
use bridge_protocol::messages::Note;
use chrono::{Datelike, Duration, NaiveDate};
use rusqlite::types::Value as Sql;
use rusqlite::Connection;
use serde_json::{json, Value};

use super::database::{self, DatabaseDoc};
use super::doc::{self, GroupByDef, PropertyDef, PropertyType, SortDef};
use super::links;
use super::service::{invalid, note_from_row, NOTE_COLUMNS};
use crate::BridgeError;

pub const MAX_CONDITIONS: usize = 20;
pub const MAX_SORTS: usize = 5;
pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;
pub const DEFAULT_GROUP_LIMIT: usize = 50;
pub const MAX_GROUP_LIMIT: usize = 200;
pub const KINDS: [&str; 5] = ["note", "idea", "task", "question", "database"];
pub const STATES: [&str; 5] = ["inbox", "open", "doing", "done", "archived"];
pub const WITHIN: [&str; 6] = ["pastWeek", "nextWeek", "thisWeek", "thisMonth", "pastMonth", "nextMonth"];

/// What a caller asks of one database. Absent fields fall back to the saved
/// view when `view_id` names one.
#[derive(Debug, Clone, Default)]
pub struct QuerySpec {
    pub view_id: Option<String>,
    pub filter: Option<Value>,
    /// Conditions a caller adds around the filter (a tool's `kind` or `state`,
    /// archived pages left out). Each compiles on its own, so they cost the
    /// filter neither a level nor a condition.
    pub constraints: Vec<Value>,
    pub sorts: Option<Vec<SortDef>>,
    pub group_by: Option<GroupByDef>,
    pub group_limit: Option<usize>,
    pub query: Option<String>,
    /// The caller's local day, `YYYY-MM-DD`; the UTC day when absent.
    pub today: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

/// One matching row: the page and its cells by property id.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub note: Note,
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: String,
    pub label: String,
    pub count: usize,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub rows: Vec<Row>,
    pub groups: Option<Vec<Group>>,
    pub total: usize,
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// What a property id names: a schema property or a page column.
#[derive(Debug, Clone)]
enum Prop<'a> {
    Value(&'a str, &'a PropertyDef),
    System(System),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum System {
    Title,
    Kind,
    State,
    Tags,
    Project,
    Chats,
    Parent,
    Created,
    Updated,
    CreatedBy,
}

impl System {
    fn id(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Kind => "kind",
            Self::State => "state",
            Self::Tags => "tags",
            Self::Project => "project",
            Self::Chats => "chats",
            Self::Parent => "parent",
            Self::Created => "created",
            Self::Updated => "updated",
            Self::CreatedBy => "createdBy",
        }
    }

    fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "title" => Self::Title,
            "kind" => Self::Kind,
            "state" => Self::State,
            "tags" => Self::Tags,
            "project" => Self::Project,
            "chats" => Self::Chats,
            "parent" => Self::Parent,
            "created" => Self::Created,
            "updated" => Self::Updated,
            "createdBy" => Self::CreatedBy,
            _ => return None,
        })
    }
}

/// The comparison family a property belongs to, which fixes its ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Text,
    Number,
    Date,
    Choice,
    List,
    Checkbox,
    Chats,
    Parent,
}

impl Family {
    fn ops(self, status: bool) -> &'static [&'static str] {
        match self {
            Family::Text => &["contains", "notContains", "equals", "notEquals", "startsWith", "isEmpty", "isNotEmpty"],
            Family::Number => &["eq", "ne", "lt", "lte", "gt", "gte", "isEmpty", "isNotEmpty"],
            Family::Date => &["before", "after", "onOrBefore", "onOrAfter", "equals", "isEmpty", "isNotEmpty", "within"],
            Family::Choice if status => &["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty", "inGroup"],
            Family::Choice => &["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty"],
            Family::List => &["contains", "notContains", "containsAny", "isEmpty", "isNotEmpty"],
            Family::Checkbox => &["is"],
            Family::Chats => &["isEmpty", "isNotEmpty"],
            Family::Parent => &["isEmpty", "isNotEmpty", "equals", "notEquals"],
        }
    }
}

impl Prop<'_> {
    fn family(&self) -> Family {
        match self {
            Prop::Value(_, def) => match def.kind {
                PropertyType::Text | PropertyType::Url => Family::Text,
                PropertyType::Number => Family::Number,
                PropertyType::Date => Family::Date,
                PropertyType::Select | PropertyType::Status => Family::Choice,
                PropertyType::MultiSelect | PropertyType::Relation => Family::List,
                PropertyType::Checkbox => Family::Checkbox,
            },
            Prop::System(system) => match system {
                System::Title | System::CreatedBy => Family::Text,
                System::Kind | System::State | System::Project => Family::Choice,
                System::Tags => Family::List,
                System::Chats => Family::Chats,
                System::Parent => Family::Parent,
                System::Created | System::Updated => Family::Date,
            },
        }
    }

    fn is_status(&self) -> bool {
        matches!(self, Prop::Value(_, def) if def.kind == PropertyType::Status)
    }

    fn name(&self) -> String {
        match self {
            Prop::Value(_, def) => def.name.clone(),
            Prop::System(system) => system.id().to_owned(),
        }
    }
}

fn resolve<'a>(database: &'a DatabaseDoc, id: &str) -> Result<Prop<'a>, BridgeError> {
    if let Some((key, def)) = database.schema.get_key_value(id) {
        return Ok(Prop::Value(key.as_str(), def));
    }
    if let Some(system) = System::parse(id) {
        return Ok(Prop::System(system));
    }
    let mut names: Vec<String> = database.ordered().into_iter().map(|(id, def)| format!("{} ({id})", def.name)).collect();
    names.extend(["title", "kind", "state", "tags", "project", "chats", "parent", "created", "updated", "createdBy"].map(str::to_owned));
    Err(invalid(format!("no property {id}; this database has {}", names.join(", "))))
}

// ---------------------------------------------------------------------------
// SQL building
// ---------------------------------------------------------------------------

/// A statement under construction with its positional parameters.
#[derive(Default)]
struct Statement {
    args: Vec<Sql>,
}

impl Statement {
    fn bind(&mut self, value: impl Into<Sql>) -> String {
        self.args.push(value.into());
        format!("?{}", self.args.len())
    }

    fn list(&mut self, values: &[String]) -> String {
        values.iter().map(|value| self.bind(value.clone())).collect::<Vec<_>>().join(", ")
    }
}

/// The context every condition compiles in.
struct Scope<'a> {
    space: &'a str,
    database: &'a DatabaseDoc,
    today: NaiveDate,
}

impl Scope<'_> {
    /// `pages.id IN (SELECT page_id FROM page_values …)` for one property.
    fn values_in(&self, sql: &mut Statement, prop: &str, predicate: &str) -> String {
        let space = sql.bind(self.space.to_owned());
        let database = sql.bind(self.database.id.clone());
        let prop = sql.bind(prop.to_owned());
        format!("pages.id IN (SELECT page_id FROM page_values WHERE space_id={space} AND database_id={database} AND prop_id={prop} AND {predicate})")
    }
}

fn not(expression: String) -> String {
    format!("NOT COALESCE(({expression}), 0)")
}

fn text_value(op: &str, value: Option<&Value>, name: &str) -> Result<String, BridgeError> {
    value
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("{op} on {name} takes text")))
}

fn text_list(op: &str, value: Option<&Value>, name: &str) -> Result<Vec<String>, BridgeError> {
    let items = value.and_then(Value::as_array).ok_or_else(|| invalid(format!("{op} on {name} takes a list")))?;
    items
        .iter()
        .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| invalid(format!("{op} on {name} takes a list of text"))))
        .collect()
}

/// A filter value as the UTC day it stands for.
fn day_of(op: &str, value: Option<&Value>, name: &str) -> Result<NaiveDate, BridgeError> {
    let raw = value.and_then(Value::as_str).ok_or_else(|| invalid(format!("{op} on {name} takes a date")))?;
    database::instant(raw)
        .map(|at| at.date_naive())
        .ok_or_else(|| invalid(format!("{op} on {name} takes YYYY-MM-DD or an RFC 3339 time, not {raw}")))
}

fn day(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The inclusive days a `within` value covers.
pub fn within_range(value: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    Some(match value {
        "pastWeek" => (today - Duration::days(7), today),
        "nextWeek" => (today, today + Duration::days(7)),
        "thisWeek" => {
            let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            (monday, monday + Duration::days(6))
        }
        "thisMonth" => {
            let first = today.with_day(1)?;
            let next = if first.month() == 12 { NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)? };
            (first, next - Duration::days(1))
        }
        "pastMonth" => (today - Duration::days(30), today),
        "nextMonth" => (today, today + Duration::days(30)),
        _ => return None,
    })
}

/// A day comparison on a column of `YYYY-MM-DD` or RFC 3339 strings. Every
/// form is a range on the raw column, so an index can answer it.
fn day_predicate(sql: &mut Statement, column: &str, op: &str, value: Option<&Value>, name: &str, today: NaiveDate) -> Result<String, BridgeError> {
    let next = |date: NaiveDate| day(date + Duration::days(1));
    Ok(match op {
        "before" => format!("{column} < {}", sql.bind(day(day_of(op, value, name)?))),
        "after" => format!("{column} >= {}", sql.bind(next(day_of(op, value, name)?))),
        "onOrBefore" => format!("{column} < {}", sql.bind(next(day_of(op, value, name)?))),
        "onOrAfter" => format!("{column} >= {}", sql.bind(day(day_of(op, value, name)?))),
        "equals" => {
            let date = day_of(op, value, name)?;
            format!("({column} >= {} AND {column} < {})", sql.bind(day(date)), sql.bind(next(date)))
        }
        "within" => {
            let raw = value.and_then(Value::as_str).unwrap_or_default();
            let (from, to) = within_range(raw, today).ok_or_else(|| invalid(format!("within takes one of {}", WITHIN.join(", "))))?;
            format!("({column} >= {} AND {column} < {})", sql.bind(day(from)), sql.bind(next(to)))
        }
        _ => unreachable!("ops are checked before compiling"),
    })
}

/// The option ids a choice value names, refused when one is not an option.
fn options_of(prop: &Prop<'_>, op: &str, value: Option<&Value>) -> Result<Vec<String>, BridgeError> {
    let name = prop.name();
    let ids = match op {
        "isAnyOf" | "isNoneOf" | "containsAny" => text_list(op, value, &name)?,
        _ => vec![text_value(op, value, &name)?],
    };
    let known: Option<Vec<String>> = match prop {
        Prop::Value(_, def) if def.kind.options() => Some(def.options.iter().map(|option| option.id.clone()).collect()),
        Prop::System(System::Kind) => Some(KINDS.map(str::to_owned).to_vec()),
        Prop::System(System::State) => Some(STATES.map(str::to_owned).to_vec()),
        _ => None,
    };
    if let Some(known) = known {
        if let Some(bad) = ids.iter().find(|id| !known.contains(id)) {
            let choices = match prop {
                Prop::Value(_, def) => def.options.iter().map(|option| format!("{} ({})", option.name, option.id)).collect::<Vec<_>>().join(", "),
                _ => known.join(", "),
            };
            return Err(invalid(format!("{name} has no option {bad}; choose one of {choices}")));
        }
    }
    Ok(ids)
}

fn compile_condition(scope: &Scope<'_>, sql: &mut Statement, condition: &serde_json::Map<String, Value>) -> Result<String, BridgeError> {
    let id = condition.get("property").and_then(Value::as_str).ok_or_else(|| invalid("a condition needs a property"))?;
    let op = condition.get("op").and_then(Value::as_str).ok_or_else(|| invalid("a condition needs an op"))?;
    if let Some(extra) = condition.keys().find(|key| !matches!(key.as_str(), "property" | "op" | "value")) {
        return Err(invalid(format!("a condition has property, op and value, not {extra}")));
    }
    let value = condition.get("value");
    let prop = resolve(scope.database, id)?;
    let family = prop.family();
    let ops = family.ops(prop.is_status());
    if !ops.contains(&op) {
        return Err(invalid(format!("{} takes {}, not {op}", prop.name(), ops.join(", "))));
    }
    let name = prop.name();
    // Negations are NOT of their positive op, so an empty cell matches them.
    let (positive, negate) = match op {
        "notContains" => ("contains", true),
        "notEquals" => ("equals", true),
        "ne" => ("eq", true),
        "isNot" => ("is", true),
        "isNoneOf" => ("isAnyOf", true),
        "isEmpty" => ("isNotEmpty", true),
        other => (other, false),
    };
    let expression = match prop {
        Prop::Value(prop_id, def) => {
            let column = match family {
                Family::Number => "num_v",
                Family::Date => "date_v",
                _ => "text_v",
            };
            match (family, positive) {
                (_, "isNotEmpty") => scope.values_in(sql, prop_id, &format!("{column} IS NOT NULL AND {column} <> ''")),
                (Family::Text, "contains") => {
                    let needle = sql.bind(text_value(op, value, &name)?);
                    scope.values_in(sql, prop_id, &format!("text_v <> '' AND instr(lower(text_v), lower({needle})) > 0"))
                }
                (Family::Text, "equals") => {
                    let needle = sql.bind(text_value(op, value, &name)?);
                    scope.values_in(sql, prop_id, &format!("text_v <> '' AND lower(text_v) = lower({needle})"))
                }
                (Family::Text, "startsWith") => {
                    let needle = sql.bind(text_value(op, value, &name)?);
                    scope.values_in(sql, prop_id, &format!("text_v <> '' AND instr(lower(text_v), lower({needle})) = 1"))
                }
                (Family::Number, op) => {
                    let number = value.and_then(Value::as_f64).filter(|number| number.is_finite()).ok_or_else(|| invalid(format!("{op} on {name} takes a number")))?;
                    let symbol = match op {
                        "eq" => "=",
                        "lt" => "<",
                        "lte" => "<=",
                        "gt" => ">",
                        _ => ">=",
                    };
                    let bound = sql.bind(number);
                    scope.values_in(sql, prop_id, &format!("num_v {symbol} {bound}"))
                }
                (Family::Date, _) => {
                    let predicate = day_predicate(sql, "date_v", positive, value, &name, scope.today)?;
                    scope.values_in(sql, prop_id, &predicate)
                }
                (Family::Choice, "is") | (Family::Choice, "isAnyOf") | (Family::List, "contains") | (Family::List, "containsAny") => {
                    let ids = if def.kind == PropertyType::Relation {
                        match positive {
                            "containsAny" => text_list(op, value, &name)?,
                            _ => vec![text_value(op, value, &name)?],
                        }
                    } else {
                        options_of(&prop, positive, value)?
                    };
                    if ids.is_empty() {
                        "0".to_owned()
                    } else {
                        let list = sql.list(&ids);
                        scope.values_in(sql, prop_id, &format!("text_v IN ({list})"))
                    }
                }
                (Family::Choice, "inGroup") => {
                    let group = text_value(op, value, &name)?;
                    if !database::STATUS_GROUPS.contains(&group.as_str()) {
                        return Err(invalid(format!("inGroup takes one of {}", database::STATUS_GROUPS.join(", "))));
                    }
                    let ids: Vec<String> = def.options.iter().filter(|option| option.group.as_deref() == Some(group.as_str())).map(|option| option.id.clone()).collect();
                    if ids.is_empty() {
                        "0".to_owned()
                    } else {
                        let list = sql.list(&ids);
                        scope.values_in(sql, prop_id, &format!("text_v IN ({list})"))
                    }
                }
                (Family::Checkbox, "is") => {
                    let checked = value.and_then(Value::as_bool).ok_or_else(|| invalid(format!("is on {name} takes true or false")))?;
                    let expression = scope.values_in(sql, prop_id, "text_v = 'true'");
                    if checked { expression } else { not(expression) }
                }
                _ => unreachable!("ops are checked before compiling"),
            }
        }
        Prop::System(system) => match system {
            System::Title | System::CreatedBy => {
                let column = if system == System::Title { "COALESCE(pages.title, '')" } else { "pages.created_by" };
                match positive {
                    "isNotEmpty" => format!("{column} <> ''"),
                    "contains" => format!("({column} <> '' AND instr(lower({column}), lower({})) > 0)", sql.bind(text_value(op, value, &name)?)),
                    "equals" => format!("({column} <> '' AND lower({column}) = lower({}))", sql.bind(text_value(op, value, &name)?)),
                    "startsWith" => format!("({column} <> '' AND instr(lower({column}), lower({})) = 1)", sql.bind(text_value(op, value, &name)?)),
                    _ => unreachable!(),
                }
            }
            System::Kind | System::State | System::Project => {
                let column = match system {
                    System::Kind => "pages.kind",
                    System::State => "pages.state",
                    _ => "pages.workspace_id",
                };
                match positive {
                    "isNotEmpty" => format!("({column} IS NOT NULL AND {column} <> '')"),
                    _ => {
                        let ids = options_of(&prop, positive, value)?;
                        if ids.is_empty() {
                            "0".to_owned()
                        } else {
                            format!("{column} IN ({})", sql.list(&ids))
                        }
                    }
                }
            }
            System::Tags => match positive {
                "isNotEmpty" => "pages.tags <> '[]'".to_owned(),
                _ => {
                    // Tags are stored trimmed, without `#`, folded; a value is
                    // read the same way, as the notes list reads one.
                    let tags = super::service::normalize_tags(&match positive {
                        "containsAny" => text_list(op, value, &name)?,
                        _ => vec![text_value(op, value, &name)?],
                    });
                    if tags.is_empty() {
                        "0".to_owned()
                    } else {
                        format!("EXISTS (SELECT 1 FROM json_each(pages.tags) WHERE value IN ({}))", sql.list(&tags))
                    }
                }
            },
            System::Chats => "pages.session_ids <> '[]'".to_owned(),
            System::Parent => match positive {
                "isNotEmpty" => "pages.parent_id IS NOT NULL".to_owned(),
                _ => format!("pages.parent_id = {}", sql.bind(text_value(op, value, &name)?)),
            },
            System::Created | System::Updated => {
                let column = if system == System::Created { "pages.created_at" } else { "pages.updated_at" };
                match positive {
                    "isNotEmpty" => format!("({column} IS NOT NULL AND {column} <> '')"),
                    _ => day_predicate(sql, column, positive, value, &name, scope.today)?,
                }
            }
        },
    };
    Ok(if negate { not(expression) } else { format!("COALESCE(({expression}), 0)") })
}

/// Compile a filter tree, counting conditions and depth on the way.
fn compile_filter(scope: &Scope<'_>, sql: &mut Statement, filter: &Value, depth: usize, count: &mut usize) -> Result<String, BridgeError> {
    let object = filter.as_object().ok_or_else(|| invalid("a filter is {and: [...]}, {or: [...]} or {property, op, value}"))?;
    for (key, joiner) in [("and", " AND "), ("or", " OR ")] {
        if let Some(items) = object.get(key) {
            if object.len() != 1 {
                return Err(invalid(format!("a {key} group holds only its list")));
            }
            if depth >= 2 {
                return Err(invalid("filter groups nest at most two levels"));
            }
            let items = items.as_array().ok_or_else(|| invalid(format!("{key} takes a list")))?;
            if items.is_empty() {
                return Ok("1".to_owned());
            }
            let parts: Vec<String> = items.iter().map(|item| compile_filter(scope, sql, item, depth + 1, count)).collect::<Result<_, _>>()?;
            return Ok(format!("({})", parts.join(joiner)));
        }
    }
    *count += 1;
    if *count > MAX_CONDITIONS {
        return Err(invalid(format!("a filter holds at most {MAX_CONDITIONS} conditions")));
    }
    compile_condition(scope, sql, object)
}

// ---------------------------------------------------------------------------
// Sorts and groups
// ---------------------------------------------------------------------------

/// A scalar `page_values` lookup for one row, through the primary key.
fn cell(scope: &Scope<'_>, sql: &mut Statement, prop: &str, column: &str) -> String {
    let space = sql.bind(scope.space.to_owned());
    let prop = sql.bind(prop.to_owned());
    format!("(SELECT {column} FROM page_values WHERE space_id={space} AND page_id=pages.id AND prop_id={prop} AND item=0)")
}

fn fixed_order(sql: &mut Statement, column: &str, values: &[String]) -> String {
    let arms: Vec<String> = values.iter().enumerate().map(|(index, value)| format!("WHEN {} THEN {index}", sql.bind(value.clone()))).collect();
    if arms.is_empty() {
        "NULL".to_owned()
    } else {
        format!("(CASE {column} {} END)", arms.join(" "))
    }
}

/// The SQL expression a sort orders by. NULL is an empty value.
fn sort_key(scope: &Scope<'_>, sql: &mut Statement, id: &str) -> Result<String, BridgeError> {
    let prop = resolve(scope.database, id)?;
    Ok(match prop {
        Prop::Value(prop_id, def) => match def.kind {
            PropertyType::Text | PropertyType::Url => format!("lower(NULLIF({}, ''))", cell(scope, sql, prop_id, "text_v")),
            PropertyType::Number => cell(scope, sql, prop_id, "num_v"),
            PropertyType::Date => cell(scope, sql, prop_id, "date_v"),
            PropertyType::Checkbox => format!("COALESCE({}, 'false')", cell(scope, sql, prop_id, "text_v")),
            PropertyType::Select | PropertyType::Status => {
                let ids: Vec<String> = def.options.iter().map(|option| option.id.clone()).collect();
                let column = cell(scope, sql, prop_id, "text_v");
                fixed_order(sql, &column, &ids)
            }
            PropertyType::MultiSelect | PropertyType::Relation => return Err(invalid(format!("{} holds a list and cannot be sorted", def.name))),
        },
        Prop::System(system) => match system {
            System::Title => "lower(NULLIF(pages.title, ''))".to_owned(),
            System::CreatedBy => "lower(NULLIF(pages.created_by, ''))".to_owned(),
            System::Kind => fixed_order(sql, "pages.kind", &KINDS.map(str::to_owned)),
            System::State => fixed_order(sql, "pages.state", &STATES.map(str::to_owned)),
            System::Project => "pages.workspace_id".to_owned(),
            System::Created => "pages.created_at".to_owned(),
            System::Updated => "pages.updated_at".to_owned(),
            System::Tags | System::Chats | System::Parent => return Err(invalid(format!("{id} cannot be sorted"))),
        },
    })
}

/// How a grouping reads a row's key or keys.
#[derive(Debug, Clone)]
enum Grouping {
    /// One key; every key in display order up front, and whether a row with
    /// none of them has a "No value" group.
    Fixed(Vec<(String, String)>, bool),
    /// One key per item, with the options listed up front.
    Items(Vec<(String, String)>),
    /// Keys as they occur, ascending.
    Occurring,
    /// A date, bucketed.
    Dates(String),
}

fn group_key(scope: &Scope<'_>, sql: &mut Statement, group: &GroupByDef) -> Result<(String, Grouping), BridgeError> {
    let prop = resolve(scope.database, &group.property)?;
    let bucket = || -> Result<String, BridgeError> {
        let bucket = group.date_bucket.clone().unwrap_or_else(|| "day".to_owned());
        if !matches!(bucket.as_str(), "day" | "week" | "month") {
            return Err(invalid("dateBucket is day, week or month"));
        }
        Ok(bucket)
    };
    Ok(match prop {
        Prop::Value(prop_id, def) => match def.kind {
            PropertyType::Select | PropertyType::Status => (
                cell(scope, sql, prop_id, "text_v"),
                Grouping::Fixed(def.options.iter().map(|option| (option.id.clone(), option.name.clone())).collect(), true),
            ),
            PropertyType::MultiSelect => {
                let space = sql.bind(scope.space.to_owned());
                let prop = sql.bind(prop_id.to_owned());
                (
                    format!("(SELECT json_group_array(text_v) FROM (SELECT text_v FROM page_values WHERE space_id={space} AND page_id=pages.id AND prop_id={prop} ORDER BY item))"),
                    Grouping::Items(def.options.iter().map(|option| (option.id.clone(), option.name.clone())).collect()),
                )
            }
            PropertyType::Checkbox => (
                format!("COALESCE({}, 'false')", cell(scope, sql, prop_id, "text_v")),
                Grouping::Fixed(vec![("true".into(), "Checked".into()), ("false".into(), "Unchecked".into())], false),
            ),
            PropertyType::Date => (cell(scope, sql, prop_id, "date_v"), Grouping::Dates(bucket()?)),
            _ => return Err(invalid(format!("{} cannot be grouped; group by a select, status, multi-select, checkbox or date", def.name))),
        },
        Prop::System(system) => match system {
            System::Kind => ("pages.kind".to_owned(), Grouping::Fixed(KINDS.map(|kind| (kind.to_owned(), title_case(kind))).to_vec(), true)),
            System::State => ("pages.state".to_owned(), Grouping::Fixed(STATES.map(|state| (state.to_owned(), title_case(state))).to_vec(), true)),
            System::Project => ("pages.workspace_id".to_owned(), Grouping::Occurring),
            System::Created => ("pages.created_at".to_owned(), Grouping::Dates(bucket()?)),
            System::Updated => ("pages.updated_at".to_owned(), Grouping::Dates(bucket()?)),
            _ => return Err(invalid(format!("{} cannot be grouped", group.property))),
        },
    })
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// A date's bucket key: its day, the Monday of its ISO week, or `YYYY-MM`.
pub fn bucket_of(raw: &str, bucket: &str) -> Option<String> {
    let date = NaiveDate::parse_from_str(raw.get(..10)?, "%Y-%m-%d").ok()?;
    Some(match bucket {
        "week" => day(date - Duration::days(date.weekday().num_days_from_monday() as i64)),
        "month" => date.format("%Y-%m").to_string(),
        _ => day(date),
    })
}

// ---------------------------------------------------------------------------
// Ordering and the cursor
// ---------------------------------------------------------------------------

/// One matching row as SQL ordered it.
#[derive(Debug, Clone)]
struct Ordered {
    id: String,
    updated: String,
    keys: Vec<Sql>,
    group: Option<Sql>,
}

fn compare_key(left: &Sql, right: &Sql, descending: bool) -> Ordering {
    let rank = |value: &Sql| matches!(value, Sql::Null);
    match (rank(left), rank(right)) {
        (true, true) => return Ordering::Equal,
        // Empty sorts last in both directions.
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let ordering = match (left, right) {
        (Sql::Integer(a), Sql::Integer(b)) => a.cmp(b),
        (Sql::Real(a), Sql::Real(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Sql::Integer(a), Sql::Real(b)) => (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal),
        (Sql::Real(a), Sql::Integer(b)) => a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal),
        (Sql::Text(a), Sql::Text(b)) => a.as_bytes().cmp(b.as_bytes()),
        _ => Ordering::Equal,
    };
    if descending { ordering.reverse() } else { ordering }
}

/// The total order SQL sorted by: the sort keys, then `updated desc, id desc`.
fn compare(left: (&[Sql], &str, &str), right: (&[Sql], &str, &str), directions: &[bool]) -> Ordering {
    for ((a, b), descending) in left.0.iter().zip(right.0.iter()).zip(directions) {
        let ordering = compare_key(a, b, *descending);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    right.1.cmp(left.1).then_with(|| right.2.cmp(left.2))
}

fn sql_json(value: &Sql) -> Value {
    match value {
        Sql::Null => Value::Null,
        Sql::Integer(number) => json!(number),
        Sql::Real(number) => json!(number),
        Sql::Text(text) => json!(text),
        Sql::Blob(_) => Value::Null,
    }
}

fn json_sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Number(number) => number.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(number.as_f64().unwrap_or_default())),
        Value::String(text) => Sql::Text(text.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn encode_cursor(row: &Ordered) -> String {
    let body = json!({"k": row.keys.iter().map(sql_json).collect::<Vec<_>>(), "u": row.updated, "i": row.id});
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body.to_string())
}

fn decode_cursor(cursor: &str, sorts: usize) -> Result<(Vec<Sql>, String, String), BridgeError> {
    let bad = || invalid("this cursor does not belong to this query");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cursor.trim()).map_err(|_| bad())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let keys: Vec<Sql> = body.get("k").and_then(Value::as_array).ok_or_else(bad)?.iter().map(json_sql).collect();
    if keys.len() != sorts {
        return Err(bad());
    }
    let updated = body.get("u").and_then(Value::as_str).ok_or_else(bad)?.to_owned();
    let id = body.get("i").and_then(Value::as_str).ok_or_else(bad)?.to_owned();
    Ok((keys, updated, id))
}

// ---------------------------------------------------------------------------
// The query
// ---------------------------------------------------------------------------

/// Validate a view's filter, sorts and grouping against a database without
/// running it. `save_view` calls this so a saved view always compiles.
pub fn check_view(space: &str, database: &DatabaseDoc, filter: Option<&Value>, sorts: &[SortDef], group_by: Option<&GroupByDef>) -> Result<(), BridgeError> {
    let today = chrono::Utc::now().date_naive();
    let scope = Scope { space, database, today };
    let mut sql = Statement::default();
    if let Some(filter) = filter {
        compile_filter(&scope, &mut sql, filter, 0, &mut 0)?;
    }
    check_sorts(sorts)?;
    for sort in sorts {
        sort_key(&scope, &mut sql, &sort.property)?;
    }
    if let Some(group) = group_by {
        group_key(&scope, &mut sql, group)?;
    }
    Ok(())
}

fn check_sorts(sorts: &[SortDef]) -> Result<(), BridgeError> {
    if sorts.len() > MAX_SORTS {
        return Err(invalid(format!("a query sorts by at most {MAX_SORTS} properties")));
    }
    if let Some(sort) = sorts.iter().find(|sort| !matches!(sort.direction.as_str(), "asc" | "desc")) {
        return Err(invalid(format!("a sort direction is asc or desc, not {}", sort.direction)));
    }
    Ok(())
}

/// The compiled statement and the shape of what it returns.
struct Plan {
    sql: String,
    args: Vec<Sql>,
    directions: Vec<bool>,
    grouping: Option<Grouping>,
}

fn plan(space: &str, database: &DatabaseDoc, spec: &QuerySpec) -> Result<Plan, BridgeError> {
    let view = match &spec.view_id {
        Some(view_id) => Some(database.views.get(view_id).cloned().ok_or_else(|| {
            let names: Vec<String> = database.views.iter().map(|(id, view)| format!("{} ({id})", view.name)).collect();
            invalid(format!("no view {view_id}; this database has {}", names.join(", ")))
        })?),
        None => None,
    };
    let filter = spec.filter.clone().or_else(|| view.as_ref().and_then(|view| view.filter.clone()));
    let sorts = spec.sorts.clone().or_else(|| view.as_ref().map(|view| view.sorts.clone())).unwrap_or_default();
    let group_by = spec.group_by.clone().or_else(|| view.as_ref().and_then(|view| view.group_by.clone()));
    check_sorts(&sorts)?;
    let today = match &spec.today {
        Some(raw) => NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| invalid(format!("today is YYYY-MM-DD, not {raw}")))?,
        None => chrono::Utc::now().date_naive(),
    };
    let scope = Scope { space, database, today };
    let mut sql = Statement::default();

    let mut keys: Vec<String> = Vec::new();
    for sort in &sorts {
        keys.push(sort_key(&scope, &mut sql, &sort.property)?);
    }
    let grouping = match &group_by {
        Some(group) => {
            let (expression, grouping) = group_key(&scope, &mut sql, group)?;
            Some((expression, grouping, group.clone()))
        }
        None => None,
    };
    let mut columns = vec!["pages.id".to_owned(), "pages.updated_at".to_owned()];
    columns.extend(keys.iter().cloned());
    if let Some((expression, _, _)) = &grouping {
        columns.push(expression.clone());
    }

    let space_arg = sql.bind(space.to_owned());
    let mut conditions = vec![format!("pages.space_id={space_arg}"), "pages.deleted_at IS NULL".to_owned()];
    if database.scope == "space" {
        let me = sql.bind(database.id.clone());
        conditions.push(format!("pages.id <> {me}"));
    } else {
        let parent = sql.bind(database.id.clone());
        conditions.push(format!("pages.parent_id = {parent}"));
    }
    if let Some(filter) = &filter {
        conditions.push(compile_filter(&scope, &mut sql, filter, 0, &mut 0)?);
    }
    for constraint in &spec.constraints {
        conditions.push(compile_filter(&scope, &mut sql, constraint, 0, &mut 0)?);
    }
    if let Some(fts) = spec.query.as_deref().map(str::trim).filter(|query| !query.is_empty()).and_then(links::fts_query) {
        let fts = sql.bind(fts);
        conditions.push(format!("pages.id IN (SELECT page_id FROM pages_fts WHERE pages_fts MATCH {fts})"));
    }
    let mut order: Vec<String> = Vec::new();
    for (index, sort) in sorts.iter().enumerate() {
        let column = format!("k{index}");
        order.push(format!("({column} IS NULL)"));
        order.push(format!("{column} {}", if sort.direction == "desc" { "DESC" } else { "ASC" }));
    }
    order.push("pages.updated_at DESC".to_owned());
    order.push("pages.id DESC".to_owned());
    let selected: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| match index {
            0 | 1 => column.clone(),
            index if index - 2 < keys.len() => format!("{column} AS k{}", index - 2),
            _ => format!("{column} AS g"),
        })
        .collect();
    let statement = format!("SELECT {} FROM pages WHERE {} ORDER BY {}", selected.join(", "), conditions.join(" AND "), order.join(", "));
    Ok(Plan {
        sql: statement,
        args: sql.args,
        directions: sorts.iter().map(|sort| sort.direction == "desc").collect(),
        grouping: grouping.map(|(_, grouping, _)| grouping),
    })
}

/// `EXPLAIN QUERY PLAN` for a spec, for the index tests.
#[cfg(test)]
fn explain(db: &Connection, space: &str, database: &DatabaseDoc, spec: &QuerySpec) -> Result<Vec<String>, BridgeError> {
    let plan = plan(space, database, spec)?;
    let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {}", plan.sql))?;
    let details = statement.query_map(rusqlite::params_from_iter(plan.args.iter()), |row| row.get::<_, String>(3))?.collect::<Result<_, _>>()?;
    Ok(details)
}

/// Run a query against one database.
pub fn run(db: &Connection, space: &str, database: &DatabaseDoc, spec: &QuerySpec) -> Result<QueryResult, BridgeError> {
    let plan = plan(space, database, spec)?;
    let sorts = plan.directions.len();
    let grouped = plan.grouping.is_some();
    let mut statement = db.prepare(&plan.sql)?;
    let ordered: Vec<Ordered> = statement
        .query_map(rusqlite::params_from_iter(plan.args.iter()), |row| {
            let keys = (0..sorts).map(|index| row.get::<_, Sql>(2 + index)).collect::<rusqlite::Result<Vec<_>>>()?;
            let group = if grouped { Some(row.get::<_, Sql>(2 + sorts)?) } else { None };
            Ok(Ordered { id: row.get(0)?, updated: row.get(1)?, keys, group })
        })?
        .collect::<Result<_, _>>()?;
    let total = ordered.len();

    let limit = spec.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let start = match &spec.cursor {
        Some(cursor) => {
            let (keys, updated, id) = decode_cursor(cursor, sorts)?;
            ordered
                .iter()
                .position(|row| compare((&row.keys, &row.updated, &row.id), (&keys, &updated, &id), &plan.directions) == Ordering::Greater)
                .unwrap_or(total)
        }
        None => 0,
    };
    let page: Vec<&Ordered> = ordered.iter().skip(start).take(limit).collect();
    let next_cursor = (start + page.len() < total).then(|| page.last().map(|row| encode_cursor(row))).flatten();

    let groups = match &plan.grouping {
        Some(grouping) => Some(bucket_rows(&ordered, grouping, spec.group_limit.unwrap_or(DEFAULT_GROUP_LIMIT).clamp(1, MAX_GROUP_LIMIT))),
        None => None,
    };

    let mut wanted: Vec<&str> = page.iter().map(|row| row.id.as_str()).collect();
    if let Some(groups) = &groups {
        for (_, _, _, ids) in groups {
            wanted.extend(ids.iter().map(String::as_str));
        }
    }
    let rows = load_rows(db, Some(database), &wanted)?;
    let pick = |id: &str| rows.get(id).cloned();
    Ok(QueryResult {
        rows: page.iter().filter_map(|row| pick(&row.id)).collect(),
        groups: groups.map(|groups| {
            groups
                .into_iter()
                .map(|(key, label, count, ids)| Group { key, label, count, rows: ids.iter().filter_map(|id| pick(id)).collect() })
                .collect()
        }),
        total,
        next_cursor,
    })
}

/// Group the ordered rows: `(key, label, count, first ids)`, in display order.
fn bucket_rows(ordered: &[Ordered], grouping: &Grouping, group_limit: usize) -> Vec<(String, String, usize, Vec<String>)> {
    let mut buckets: Vec<(String, String, usize, Vec<String>)> = Vec::new();
    let add = |buckets: &mut Vec<(String, String, usize, Vec<String>)>, key: &str, id: &str| {
        let Some(bucket) = buckets.iter_mut().find(|bucket| bucket.0 == key) else { return };
        bucket.2 += 1;
        if bucket.3.len() < group_limit {
            bucket.3.push(id.to_owned());
        }
    };
    let text = |value: &Option<Sql>| match value {
        Some(Sql::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    match grouping {
        Grouping::Fixed(keys, _) | Grouping::Items(keys) => {
            buckets.extend(keys.iter().map(|(key, label)| (key.clone(), label.clone(), 0, Vec::new())));
            buckets.push((String::new(), "No value".into(), 0, Vec::new()));
            for row in ordered {
                let keys: Vec<String> = match grouping {
                    Grouping::Items(_) => text(&row.group)
                        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
                        .unwrap_or_default(),
                    _ => text(&row.group).into_iter().collect(),
                };
                let known: Vec<&String> = keys.iter().filter(|key| buckets.iter().any(|bucket| &bucket.0 == *key && !key.is_empty())).collect();
                if known.is_empty() {
                    add(&mut buckets, "", &row.id);
                }
                for key in known {
                    add(&mut buckets, key, &row.id);
                }
            }
            // "No value" stays for option groupings even when empty, so a board
            // always has the column. A checkbox is never empty.
            if matches!(grouping, Grouping::Fixed(_, false)) {
                buckets.retain(|bucket| !bucket.0.is_empty());
            }
        }
        Grouping::Occurring | Grouping::Dates(_) => {
            let key_of = |row: &Ordered| -> Option<String> {
                let raw = text(&row.group)?;
                match grouping {
                    Grouping::Dates(bucket) => bucket_of(&raw, bucket),
                    _ => Some(raw),
                }
            };
            let mut keys: Vec<String> = ordered.iter().filter_map(key_of).collect();
            keys.sort();
            keys.dedup();
            buckets.extend(keys.into_iter().map(|key| (key.clone(), key, 0, Vec::new())));
            buckets.push((String::new(), "No value".into(), 0, Vec::new()));
            for row in ordered {
                let key = key_of(row).unwrap_or_default();
                add(&mut buckets, &key, &row.id);
            }
            buckets.retain(|bucket| !bucket.0.is_empty() || bucket.2 > 0);
        }
    }
    buckets
}

/// Notes and their cells for the given ids, keyed by id. Only the cells of
/// properties the database still has: a deleted property's cell stays in the
/// row's doc but is not a value of the row, as on a page read. Each id's doc
/// is decoded once, however many groups it appears in.
fn load_rows(db: &Connection, database: Option<&DatabaseDoc>, ids: &[&str]) -> Result<BTreeMap<String, Row>, BridgeError> {
    let mut unique: Vec<&str> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut rows = BTreeMap::new();
    for chunk in unique.chunks(500) {
        let marks = (1..=chunk.len()).map(|index| format!("?{index}")).collect::<Vec<_>>().join(", ");
        let mut statement = db.prepare(&format!("SELECT {NOTE_COLUMNS} FROM pages WHERE id IN ({marks})"))?;
        let notes: Vec<Note> = statement.query_map(rusqlite::params_from_iter(chunk.iter()), note_from_row)?.collect::<Result<_, _>>()?;
        for note in notes {
            let mut values = doc::load_page(db, &note.id, None)?.0.values();
            if let Some(database) = database {
                values.retain(|prop, _| database.schema.contains_key(prop));
            }
            rows.insert(note.id.clone(), Row { note, values });
        }
    }
    Ok(rows)
}

#[cfg(test)]
pub(crate) mod tests;
