//! The Notes service: the one write path for people, agents and model passes.
//!
//! Every change goes through [`NotesService::write`]: rebuild the page doc,
//! make the change as one Yjs update, append it to `page_updates`, refresh the
//! `pages` index row, all in one transaction; then mirror the page to its
//! Markdown file and announce `notes/page_changed`. Nothing else writes
//! `pages` or `page_updates`, so the index can always be rebuilt from the log.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bridge_protocol::messages::{Note, NoteAppend, NoteEditResult, NoteHistory, NotePage, NotePageChanged, NotePassProgress, NoteTextEdit, NoteVersion, NoteVersionPage, NotesSettings, StructuredView};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::doc::{self, key, OptionDef, PageDoc, PropertyDef, PropertyType, PropsPatch, RecordsPatch, ViewDef};
use super::ids::{new_id, Hlc};
use super::{assets, database, jobs, links, notices, projector, store};
use crate::BridgeError;

/// Kinds a model pass may pick for a page it wrote.
pub const KINDS: [&str; 4] = ["note", "idea", "task", "question"];
/// Kinds a page may hold. `database` is person-only: a model pass never
/// chooses it.
pub const PAGE_KINDS: [&str; 5] = ["note", "idea", "task", "question", "database"];
pub const STATES: [&str; 5] = ["inbox", "open", "doing", "done", "archived"];
pub const PERSON: &str = "person";
pub const FILE_IMPORT: &str = "file-import";
/// The actor that owns the system database. Never a person or an agent.
pub const SYSTEM: &str = "system";
const PREVIEW_CHARS: usize = 280;

/// Called after every committed change.
pub type ChangeListener = Arc<dyn Fn(&NotePageChanged) + Send + Sync>;
/// Hears a model pass's reply as it streams.
pub type ProgressListener = Arc<dyn Fn(&NotePassProgress) + Send + Sync>;
/// Answers whether a Bridge workspace (project) id exists.
pub type ProjectCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub fn agent_actor(harness: &str, session_id: &str) -> String {
    format!("agent:{harness}:{session_id}")
}

pub fn model_actor(pass: &str, model: &str) -> String {
    format!("model:{pass}:{model}")
}

/// History lists at most this many versions, the newest.
const HISTORY_LIMIT: i64 = 200;

/// Characters added and removed between two bodies. The diff has a deadline,
/// so a huge rewrite reports a coarser (larger) count rather than stalling.
fn changed_chars(before: &str, after: &str) -> (u32, u32) {
    if before == after {
        return (0, 0);
    }
    let diff = similar::TextDiff::configure().timeout(std::time::Duration::from_millis(50)).diff_chars(before, after);
    let mut counts = (0u32, 0u32);
    for change in diff.iter_all_changes() {
        let chars = change.value().chars().count() as u32;
        match change.tag() {
            similar::ChangeTag::Insert => counts.0 += chars,
            similar::ChangeTag::Delete => counts.1 += chars,
            similar::ChangeTag::Equal => {}
        }
    }
    counts
}

/// The wall time an HLC stamp (`<millis>-<counter>-<device>`) was taken at.
fn hlc_time(hlc: &str) -> String {
    let millis = hlc.split('-').next().and_then(|part| part.parse::<i64>().ok()).unwrap_or(0);
    Utc.timestamp_millis_opt(millis).single().unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

pub fn body_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Default)]
pub struct CreatePage {
    pub title: Option<String>,
    pub content: Option<String>,
    pub parent_id: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub workspace_id: Option<String>,
    pub source_id: Option<String>,
    pub source_excerpt: Option<String>,
    pub state: Option<String>,
    /// A database's scope: `children` (the default) or `space`.
    pub scope: Option<String>,
    /// A database's first views; one "Table" view when empty.
    pub views: Vec<ViewDef>,
    /// A database's first properties, checked and written with the page.
    pub properties: Vec<NewProperty>,    /// A row's first cells by property id, checked and written with the page.
    pub values: Vec<(String, Option<Value>)>,
}

#[derive(Debug, Clone, Default)]
pub struct EditPage {
    pub id: String,
    pub base_seq: Option<i64>,
    pub text: Option<String>,
    pub edits: Vec<NoteTextEdit>,
    pub append: Option<NoteAppend>,
    pub allow_deleting_children: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateNote {
    pub id: String,
    pub parent_id: Option<String>,
    pub clear_parent: bool,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub workspace_id: Option<String>,
    pub clear_workspace: bool,
    pub state: Option<String>,
    pub add_session_id: Option<String>,
    /// Cells to set (`Some`) or clear (`None`) by property id, on a row.
    pub values: Vec<(String, Option<Value>)>,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub query: Option<String>,
    pub tags: Vec<String>,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub parent_id: Option<String>,
    pub include_archived: bool,
    pub limit: Option<usize>,
}

/// What `create_database` makes.
#[derive(Debug, Clone, Default)]
pub struct CreateDatabase {
    pub title: Option<String>,
    /// The database page's own words and tags, written with it.
    pub content: Option<String>,
    pub tags: Vec<String>,
    pub parent_id: Option<String>,
    pub workspace_id: Option<String>,
    pub properties: Vec<NewProperty>,
    pub views: Vec<ViewDef>,
}

/// Most views one database holds.
pub const MAX_VIEWS: usize = 50;

/// One option of a `SetOptions` list. `color` and `group` left out keep what
/// an existing option had.
#[derive(Debug, Clone, Default)]
pub struct OptionInput {
    pub id: Option<String>,
    pub name: String,
    pub color: Option<String>,
    pub group: Option<String>,
}

/// One option, as a person adds it.
#[derive(Debug, Clone, Default)]
pub struct NewOption {
    pub name: String,
    pub color: Option<String>,
    /// `status` only.
    pub group: Option<String>,
}

/// A property to add to a database.
#[derive(Debug, Clone, Default)]
pub struct NewProperty {
    pub name: String,
    pub kind: String,
    pub options: Vec<NewOption>,
    pub relation: Option<String>,
    pub date_format: Option<String>,
}

/// One change to a database's schema.
#[derive(Debug, Clone)]
pub enum SchemaOp {
    Add(NewProperty),
    Rename { prop: String, name: String },
    Delete { prop: String },
    /// Converts every row's value where it can; anything else is cleared.
    Retype { prop: String, to: String },
    AddOption { prop: String, name: String, color: Option<String>, group: Option<String> },
    /// Replace a property's whole option list. An entry without an id is new,
    /// an id left out is deleted (its values cleared), and order is list order.
    /// `base`, when given, is the option ids (in order) the caller read: the
    /// write is refused unless they are still exactly what is stored, so two
    /// writers replacing the list never delete each other's options.
    SetOptions { prop: String, options: Vec<OptionInput>, base: Option<Vec<String>> },
    /// The schema order: every property, once.
    Reorder { props: Vec<String> },
    RenameOption { prop: String, option: String, name: String },
    DeleteOption { prop: String, option: String },
}

/// What a schema write did.
#[derive(Debug, Clone, Default)]
pub struct SchemaUpdated {
    /// Properties this call added, as (id, name), in the order they were added.
    pub added: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl SchemaOp {
    /// The property this op changes, for the batch rule.
    pub fn property(&self) -> Option<&String> {
        match self {
            Self::Add(_) | Self::Reorder { .. } => None,
            Self::Rename { prop, .. } | Self::Delete { prop } | Self::Retype { prop, .. } | Self::AddOption { prop, .. } | Self::SetOptions { prop, .. } | Self::RenameOption { prop, .. } | Self::DeleteOption { prop, .. } => Some(prop),
        }
    }

    /// An option edit changes one property's options, not its shape.
    pub fn is_option(&self) -> bool {
        matches!(self, Self::AddOption { .. } | Self::SetOptions { .. } | Self::RenameOption { .. } | Self::DeleteOption { .. })
    }
}

/// The work a validated schema batch leaves to write after the commit.
#[derive(Default)]
struct SchemaPlan {
    updated: SchemaUpdated,
    /// (row, property, old def, new def, value as it stood before the batch)
    retypes: Vec<(String, String, PropertyDef, PropertyDef, Value)>,
    /// (property, option) values to remove after a deleted option.
    dropped: Vec<(String, String)>,
    /// (property, def) whose new name may already be in the rows' files.
    imported: Vec<(String, PropertyDef)>,
    /// Old names to retire, with the property that owned them.
    retire: Vec<(String, String)>,
    /// Whether the rows' files must be rewritten with the new schema.
    rewrite: bool,
}

fn no_property(prop: &str) -> BridgeError {
    invalid(format!("no property with id {prop}"))
}

fn new_options(kind: PropertyType, options: &[NewOption]) -> Result<Vec<OptionDef>, BridgeError> {
    if !options.is_empty() && !kind.options() {
        return Err(invalid(format!("{} properties do not take options", kind.as_str())));
    }
    let mut def = PropertyDef { name: "property".into(), kind, order: 0, options: Vec::new(), relation: None, date_format: None };
    for option in options {
        let name = database::check_option_name(&def, &option.name, None)?;
        let group = option_group(kind, &option.group)?;
        def.options.push(OptionDef { id: new_id(), name, color: option.color.clone(), group });
    }
    Ok(def.options)
}

/// A new property, checked against the schema it joins and placed last.
fn build_property(db: &Connection, schema: &std::collections::BTreeMap<String, PropertyDef>, new: &NewProperty) -> Result<PropertyDef, BridgeError> {
    let kind = PropertyType::parse(&new.kind).ok_or_else(|| invalid(format!("type must be one of {}", database::PROPERTY_TYPES.join(", "))))?;
    let name = database::check_name(schema, &new.name, None)?;
    let options = new_options(kind, &new.options)?;
    let relation = match &new.relation {
        Some(target) if kind == PropertyType::Relation => {
            if database::read(db, target)?.is_none() {
                return Err(invalid(format!("no database with id {target}")));
            }
            Some(doc::RelationDef { database_id: target.clone() })
        }
        Some(_) => return Err(invalid("only a relation names another database")),
        None if kind == PropertyType::Relation => return Err(invalid("a relation needs the database it points at")),
        None => None,
    };
    if let Some(format) = &new.date_format {
        if kind != PropertyType::Date || !matches!(format.as_str(), "date" | "datetime") {
            return Err(invalid("dateFormat is date or datetime, on a date property"));
        }
    }
    let order = schema.values().map(|def| def.order).max().map(|order| order + 1).unwrap_or(0);
    Ok(PropertyDef { name, kind, order, options, relation, date_format: new.date_format.clone() })
}

/// One cell a row may hold: a value its property takes, and for a relation,
/// live rows of the database it points at. `row` is the page being written
/// with the parent it has once the write lands, since the index does not
/// know a move until it commits.
fn check_cell<'a>(db: &Connection, space: &str, database: &'a database::DatabaseDoc, prop_id: &str, value: Option<&Value>, row: (&str, Option<&str>)) -> Result<&'a PropertyDef, BridgeError> {
    let def = database
        .schema
        .get(prop_id)
        .ok_or_else(|| invalid(format!("no property with id {prop_id}; this database has {}", database.property_names())))?;
    let Some(value) = value else { return Ok(def) };
    database::validate_value(def, value)?;
    if def.kind == PropertyType::Relation {
        let target = def.relation.as_ref().map(|relation| relation.database_id.clone()).ok_or_else(|| invalid(format!("{} names no database", def.name)))?;
        for id in value.as_array().into_iter().flatten().filter_map(Value::as_str) {
            let exists: bool = if id == row.0 {
                row.1 == Some(target.as_str())
            } else {
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pages WHERE id=?1 AND parent_id=?2 AND space_id=?3 AND deleted_at IS NULL)",
                    params![id, target, space],
                    |row| row.get(0),
                )?
            };
            if !exists {
                return Err(invalid(format!("{} names {id}, which is not a row of that database", def.name)));
            }
        }
    }
    Ok(def)
}

/// The records a cells patch writes, each cell checked against `database`.
fn cells_records(db: &Connection, space: &str, database: &database::DatabaseDoc, current: &std::collections::BTreeMap<String, Value>, patch: &[(String, Option<Value>)], row: (&str, Option<&str>)) -> Result<RecordsPatch, BridgeError> {
    let mut records = RecordsPatch::default();
    for (prop_id, value) in patch {
        check_cell(db, space, database, prop_id, value.as_ref(), row)?;
        match value {
            Some(value) => records = records.put_value(prop_id, Some(value)).settle_value(prop_id),
            None => {
                if let Some(value) = current.get(prop_id) {
                    records = records.clear_value(prop_id, value);
                }
                records = records.put_value(prop_id, None);
            }
        }
    }
    Ok(records)
}

/// A view whose property and option references may be names, with each name
/// of a property in `schema` (or of one of its options) swapped for its id.
fn resolve_view_names(view: &ViewDef, schema: &std::collections::BTreeMap<String, PropertyDef>) -> ViewDef {
    let fold = |text: &str| text.trim().to_lowercase();
    let property = |reference: &str| -> String {
        if schema.contains_key(reference) {
            return reference.to_owned();
        }
        schema.iter().find(|(_, def)| fold(&def.name) == fold(reference)).map(|(id, _)| id.clone()).unwrap_or_else(|| reference.to_owned())
    };
    fn filter(tree: &Value, schema: &std::collections::BTreeMap<String, PropertyDef>, property: &dyn Fn(&str) -> String) -> Value {
        if let Some(object) = tree.as_object() {
            for key in ["and", "or"] {
                if let Some(items) = object.get(key).and_then(Value::as_array) {
                    return serde_json::json!({ key: items.iter().map(|item| filter(item, schema, property)).collect::<Vec<_>>() });
                }
            }
        }
        let mut condition = tree.clone();
        let Some(name) = tree.get("property").and_then(Value::as_str) else { return condition };
        let id = property(name);
        condition["property"] = Value::String(id.clone());
        if let (Some(def), Some(value)) = (schema.get(&id).filter(|def| def.kind.options()), tree.get("value")) {
            if tree.get("op").and_then(Value::as_str) != Some("inGroup") {
                let option = |raw: &Value| match raw.as_str() {
                    Some(raw) if def.option(raw).is_none() => def.option_by_name(raw).map(|option| Value::String(option.id.clone())).unwrap_or_else(|| Value::String(raw.to_owned())),
                    _ => raw.clone(),
                };
                condition["value"] = match value {
                    Value::Array(items) => Value::Array(items.iter().map(option).collect()),
                    other => option(other),
                };
            }
        }
        condition
    }
    ViewDef {
        filter: view.filter.as_ref().map(|tree| filter(tree, schema, &property)),
        sorts: view.sorts.iter().map(|sort| doc::SortDef { property: property(&sort.property), direction: sort.direction.clone() }).collect(),
        group_by: view.group_by.as_ref().map(|group| doc::GroupByDef { property: property(&group.property), date_bucket: group.date_bucket.clone() }),
        date_property: view.date_property.as_deref().map(property),
        properties: view.properties.iter().map(|column| doc::ViewPropertyDef { id: property(&column.id), ..column.clone() }).collect(),
        ..view.clone()
    }
}

/// A view with every part the schema in `draft` cannot run taken out.
fn prune_view(space: &str, draft: &database::DatabaseDoc, view: &ViewDef) -> ViewDef {
    let runs = |filter: &Value| super::query::check_view(space, draft, Some(filter), &[], None).is_ok();
    fn prune(filter: &Value, runs: &dyn Fn(&Value) -> bool) -> Option<Value> {
        let object = filter.as_object()?;
        for key in ["and", "or"] {
            if let Some(items) = object.get(key).and_then(Value::as_array) {
                if items.is_empty() {
                    return Some(filter.clone());
                }
                let kept: Vec<Value> = items.iter().filter_map(|item| prune(item, runs)).collect();
                return (!kept.is_empty()).then(|| serde_json::json!({ key: kept }));
            }
        }
        if runs(filter) {
            return Some(filter.clone());
        }
        // A list keeps the items that still run, if any do.
        let items = filter.get("value").and_then(Value::as_array)?;
        let one = |item: &Value| {
            let mut single = filter.clone();
            single["value"] = serde_json::json!([item]);
            runs(&single)
        };
        let kept: Vec<Value> = items.iter().filter(|item| one(item)).cloned().collect();
        let mut narrowed = filter.clone();
        narrowed["value"] = Value::Array(kept.clone());
        (!kept.is_empty() && runs(&narrowed)).then_some(narrowed)
    }
    let known = |id: &str| draft.schema.contains_key(id) || database::SYSTEM_PROPERTIES.contains(&id) || database::FILTER_ONLY_PROPERTIES.contains(&id);
    ViewDef {
        filter: view.filter.as_ref().and_then(|filter| prune(filter, &runs)),
        sorts: view.sorts.iter().filter(|sort| super::query::check_view(space, draft, None, std::slice::from_ref(sort), None).is_ok()).cloned().collect(),
        group_by: view.group_by.clone().filter(|group| super::query::check_view(space, draft, None, &[], Some(group)).is_ok()),
        date_property: view.date_property.clone().filter(|date| draft.schema.get(date).is_some_and(|def| def.kind == PropertyType::Date) || matches!(date.as_str(), "created" | "updated")),
        properties: view.properties.iter().filter(|property| known(&property.id)).cloned().collect(),
        ..view.clone()
    }
}

/// A view a database can save: it compiles, and its date property is a date.
fn check_view_def(space: &str, database: &database::DatabaseDoc, view: &ViewDef) -> Result<(), BridgeError> {
    if view.name.trim().is_empty() {
        return Err(invalid("a view needs a name"));
    }
    for property in view.sorts.iter().map(|sort| &sort.property).chain(view.group_by.iter().map(|group| &group.property)).chain(view.date_property.iter()).chain(view.properties.iter().map(|property| &property.id)) {
        if !database.schema.contains_key(property) && !database::SYSTEM_PROPERTIES.contains(&property.as_str()) && !database::FILTER_ONLY_PROPERTIES.contains(&property.as_str()) {
            return Err(no_property(property));
        }
    }
    super::query::check_view(space, database, view.filter.as_ref(), &view.sorts, view.group_by.as_ref())?;
    if let Some(date) = &view.date_property {
        let is_date = database.schema.get(date).is_some_and(|def| def.kind == PropertyType::Date) || matches!(date.as_str(), "created" | "updated");
        if !is_date {
            return Err(invalid(format!("{date} is not a date property")));
        }
    }
    Ok(())
}

fn option_group(kind: PropertyType, group: &Option<String>) -> Result<Option<String>, BridgeError> {
    match (kind, group.as_deref()) {
        (PropertyType::Status, Some(group)) => {
            if !database::STATUS_GROUPS.contains(&group) {
                return Err(invalid(format!("a status group must be one of {}", database::STATUS_GROUPS.join(", "))));
            }
            Ok(Some(group.to_owned()))
        }
        (PropertyType::Status, None) => Ok(Some("todo".to_owned())),
        (_, Some(_)) => Err(invalid("only a status option has a group")),
        (_, None) => Ok(None),
    }
}

pub struct NotesService {
    db: Mutex<Connection>,
    clock: Mutex<Hlc>,
    device: String,
    space: String,
    root: Mutex<PathBuf>,
    project_exists: ProjectCheck,
    listener: Mutex<Option<ChangeListener>>,
    progress_listener: Mutex<Option<ProgressListener>>,
    uploads: assets::Uploads,
    /// Held for a whole file import, so two importers (the poll and a list)
    /// never merge the same outside edit twice.
    import_lock: Mutex<()>,
    /// Held while mirroring, so projections follow commit order: each one
    /// reads the newest committed row and the hash the previous one wrote.
    projection_lock: Mutex<()>,
    /// Held for a whole rename, from the title change to the last rewritten
    /// backlink, so overlapping renames cannot rewrite links to a stale title.
    rename_lock: Mutex<()>,
    /// Serializes accepting task extraction replies, including overlapping jobs.
    pub(super) task_lock: Mutex<()>,
}

impl NotesService {
    /// Open `notes.db` in `data_dir`, mirroring pages under `data_dir/notes`.
    pub fn open(data_dir: &Path, project_exists: ProjectCheck) -> Result<Self, BridgeError> {
        Self::open_at(&data_dir.join(store::FILE_NAME), data_dir.join("notes"), project_exists)
    }

    pub fn open_at(db_path: &Path, default_root: PathBuf, project_exists: ProjectCheck) -> Result<Self, BridgeError> {
        let connection = store::open(db_path)?;
        let device = store::device_id(&connection)?;
        let space = store::personal_space(&connection)?;
        let newest = store::newest_stamp(&connection, &device)?;
        let root = store::get_setting(&connection, "root")?.map(PathBuf::from).unwrap_or(default_root);
        jobs::recover(&connection)?;
        let service = Self {
            clock: Mutex::new(Hlc::resume(device.clone(), newest.as_deref())),
            db: Mutex::new(connection),
            device,
            space,
            root: Mutex::new(root),
            project_exists,
            listener: Mutex::new(None),
            progress_listener: Mutex::new(None),
            uploads: assets::Uploads::default(),
            import_lock: Mutex::new(()),
            projection_lock: Mutex::new(()),
            rename_lock: Mutex::new(()),
            task_lock: Mutex::new(()),
        };
        // Pages committed but never mirrored (a failed write, a crash between
        // commit and projection) get their files now.
        if let Err(error) = projector::reconcile(&service) {
            eprintln!("[notes] could not mirror pending pages: {error}");
        }
        // Every space gets one system database, the page whose views are the
        // Notes list's tabs. Idempotent: a reopened store finds it already.
        if let Err(error) = service.ensure_system_database() {
            eprintln!("[notes] could not create the system database: {error}");
        }
        Ok(service)
    }

    pub fn root(&self) -> PathBuf {
        self.root.lock().unwrap().clone()
    }

    pub fn settings(&self) -> Result<NotesSettings, BridgeError> {
        let auto = self.with_db(|db| store::get_setting(db, "auto_structure"))?;
        Ok(NotesSettings { auto_structure: auto.as_deref() == Some("true"), root: self.root().to_string_lossy().into_owned() })
    }

    /// Change settings. A new root applies to pages mirrored from now on;
    /// files already written stay where they are.
    pub fn save_settings(&self, auto_structure: Option<bool>, root: Option<String>) -> Result<NotesSettings, BridgeError> {
        if let Some(root) = root.as_deref().map(str::trim).filter(|root| !root.is_empty()) {
            let path = PathBuf::from(root);
            if !path.is_absolute() {
                return Err(invalid("the notes folder must be an absolute path"));
            }
            std::fs::create_dir_all(&path)?;
            self.with_db(|db| store::set_setting(db, "root", root))?;
            *self.root.lock().unwrap() = path;
        }
        if let Some(auto) = auto_structure {
            self.with_db(|db| store::set_setting(db, "auto_structure", if auto { "true" } else { "false" }))?;
        }
        self.settings()
    }

    /// Queue an opt-in model pass on a page.
    pub fn request_pass(&self, id: &str, kind: &str, harness: Option<&str>, model: Option<&str>) -> Result<String, BridgeError> {
        self.check_live(id)?;
        self.with_db(|db| jobs::enqueue(db, &self.space, id, kind, harness, model))
    }

    /// Make one structured item its own note, quoting where it came from.
    pub fn split_item(&self, id: &str, item_index: usize, actor: &str) -> Result<Note, BridgeError> {
        let page = self.read(id)?;
        let view = page.structured.ok_or_else(|| invalid("this note has no structured view yet"))?;
        let item = view.items.get(item_index).ok_or_else(|| invalid("no such item"))?;
        self.create(
            CreatePage {
                title: Some(item.title.clone()),
                content: Some(item.excerpt.clone()),
                kind: Some(item.kind.clone()),
                source_id: Some(id.to_owned()),
                state: Some("inbox".into()),
                ..Default::default()
            },
            actor,
        )
    }

    /// The page's versions, newest first: who wrote each, when, and how many
    /// characters it added and removed. Replays the log once, from the version
    /// before the window, so a long history costs one pass.
    pub fn history(&self, id: &str) -> Result<NoteHistory, BridgeError> {
        self.check_live(id)?;
        self.with_db(|db| {
            let first: Option<i64> = db.query_row(
                "SELECT MIN(seq) FROM (SELECT seq FROM page_updates WHERE page_id=?1 ORDER BY seq DESC LIMIT ?2)",
                params![id, HISTORY_LIMIT],
                |row| row.get(0),
            )?;
            let Some(first) = first else { return Ok(NoteHistory { versions: Vec::new() }) };
            let (page, _) = doc::load_page(db, id, Some(first - 1))?;
            let mut body = page.body();
            let mut statement = db.prepare("SELECT seq, hlc, actor, update_bytes FROM page_updates WHERE page_id=?1 AND seq>=?2 ORDER BY seq")?;
            let rows = statement.query_map(params![id, first], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?)))?;
            let mut versions = Vec::new();
            for row in rows {
                let (seq, hlc, actor, bytes) = row?;
                page.apply(&bytes)?;
                let next = page.body();
                let (added, removed) = changed_chars(&body, &next);
                versions.push(NoteVersion { seq, actor, at: hlc_time(&hlc), body_changed: next != body, added, removed, title: page.props().title });
                body = next;
            }
            versions.reverse();
            Ok(NoteHistory { versions })
        })
    }

    /// The page as it was at `seq`.
    pub fn read_version(&self, id: &str, seq: i64) -> Result<NoteVersionPage, BridgeError> {
        self.check_live(id)?;
        self.with_db(|db| {
            let (hlc, actor): (String, String) = db
                .query_row("SELECT hlc, actor FROM page_updates WHERE page_id=?1 AND seq=?2", params![id, seq], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?
                .ok_or_else(|| invalid("no such version"))?;
            let (page, _) = doc::load_page(db, id, Some(seq))?;
            Ok(NoteVersionPage { seq, actor, at: hlc_time(&hlc), title: page.props().title, markdown: page.body() })
        })
    }

    /// Write the words of version `seq` back as a new change. Properties stay
    /// as they are; the log keeps every version, so a restore can be undone by
    /// restoring again. It may drop a child tag added later: that is the
    /// person's explicit choice.
    pub fn restore_version(&self, id: &str, seq: i64, actor: &str) -> Result<NoteEditResult, BridgeError> {
        let words = self.read_version(id, seq)?.markdown;
        let (note, seq, changed) = self.write(id, actor, |live| live.set_body(&words))?;
        Ok(NoteEditResult { note, seq, warnings: if changed { Vec::new() } else { vec!["nothing changed".to_owned()] } })
    }

    pub fn space(&self) -> &str {
        &self.space
    }

    pub fn set_listener(&self, listener: ChangeListener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    pub fn set_progress_listener(&self, listener: ProgressListener) {
        *self.progress_listener.lock().unwrap() = Some(listener);
    }

    /// Pass on a model pass's reply so far. Stores nothing.
    pub fn report_progress(&self, progress: &NotePassProgress) {
        if let Some(listener) = self.progress_listener.lock().unwrap().clone() {
            listener(progress);
        }
    }

    pub(crate) fn uploads(&self) -> &assets::Uploads {
        &self.uploads
    }

    /// Run `read` against the database. For queries and the job queue; page
    /// writes go through [`Self::write`].
    pub fn with_db<T>(&self, read: impl FnOnce(&Connection) -> Result<T, BridgeError>) -> Result<T, BridgeError> {
        read(&self.db.lock().unwrap())
    }

    /// Save Your words exactly as typed. Calls no model.
    pub fn capture(&self, body: &str, actor: &str) -> Result<Note, BridgeError> {
        if body.trim().is_empty() {
            return Err(invalid("a note needs some text"));
        }
        let note = self.create(
            CreatePage { content: Some(body.to_owned()), state: Some("inbox".into()), ..Default::default() },
            actor,
        )?;
        if self.settings()?.auto_structure {
            self.request_pass(&note.id, jobs::STRUCTURE, None, None)?;
        }
        Ok(note)
    }

    pub fn create(&self, page: CreatePage, actor: &str) -> Result<Note, BridgeError> {
        let kind = page.kind.unwrap_or_else(|| "note".into());
        check_kind(&kind)?;
        let state = page.state.unwrap_or_else(|| "open".into());
        check_state(&state)?;
        if let Some(workspace) = &page.workspace_id {
            self.check_project(workspace)?;
        }
        if let Some(parent) = &page.parent_id {
            self.check_live(parent)?;
        }
        if page.scope.as_deref() == Some("space") && actor != SYSTEM {
            return Err(invalid("only Bridge owns the space-wide database"));
        }
        let title = page.title.map(|title| title.trim().to_owned()).filter(|title| !title.is_empty());
        let tags = normalize_tags(&page.tags);
        let id = new_id();
        let body = page.content.unwrap_or_default();
        let space = self.space.clone();
        let space_name = self.space.clone();
        let scope = page.scope.clone().unwrap_or_else(|| "children".to_owned());
        if kind == "database" && !database::SCOPES.contains(&scope.as_str()) {
            return Err(invalid(format!("scope must be one of {}", database::SCOPES.join(", "))));
        }
        // The Notes list's seven views belong to the system database alone; a
        // database a person makes starts with one table.
        let views: Vec<(String, ViewDef)> = match kind.as_str() {
            "database" if page.views.is_empty() => vec![(new_id(), database::table_view())],
            "database" => page.views.iter().map(|view| (new_id(), ViewDef { name: view.name.trim().to_owned(), ..view.clone() })).collect(),
            _ => Vec::new(),
        };
        if kind != "database" && !page.properties.is_empty() {
            return Err(invalid("only a database has properties"));
        }
        if kind == "database" && !page.values.is_empty() {
            return Err(invalid("a database has properties, not values"));
        }
        let (note, _, _) = self.write_with(&id, actor, |db, doc, _| {
            if let Some(parent) = &page.parent_id {
                validate_parent(db, &space, &id, parent)?;
                // A row holds values from its database's schema, so a database
                // cannot also be one. `validate_parent` covers the move path.
                if kind == "database" && database::read(db, parent)?.is_some() {
                    return Err(invalid("a database cannot be a row of another database"));
                }
            }
            let patch = PropsPatch::default()
                .set(key::SPACE, space)
                .maybe(key::TITLE, title)
                .set(key::KIND, kind)
                .list(key::TAGS, &tags)
                .maybe(key::WORKSPACE, page.workspace_id)
                .set(key::STATE, state)
                .maybe(key::PARENT, page.parent_id.clone())
                .maybe(key::SOURCE, page.source_id)
                .maybe(key::EXCERPT, page.source_excerpt)
                .set(key::CREATED_BY, actor)
                .set(key::CREATED_AT, now());
            if !page.values.is_empty() {
                // A row's first cells are checked against its database here and
                // land with the page: a refused cell makes no row.
                let parent = page.parent_id.as_deref().ok_or_else(|| invalid("only a row of a database has values"))?;
                let database = database::read(db, parent)?.ok_or_else(|| invalid("only a row of a database has values"))?;
                let records = cells_records(db, &space_name, &database, &Default::default(), &page.values, (&id, Some(parent)))?;
                doc.edit_page(Some(&body), &patch, &records)
            } else if views.is_empty() {
                doc.edit(Some(&body), &patch)
            } else {
                // Properties and views are checked here, against the schema they
                // make, so the page and its records land in one write or none.
                let mut schema = std::collections::BTreeMap::new();
                for property in &page.properties {
                    let def = build_property(db, &schema, property)?;
                    schema.insert(new_id(), def);
                }
                let draft = database::DatabaseDoc { id: id.clone(), scope: scope.clone(), schema: schema.clone(), views: Default::default(), retired: Default::default() };
                let mut records = RecordsPatch::default().scope(&scope);
                for (prop, def) in &schema {
                    records = records.put_schema(prop, def);
                }
                for (view_id, view) in &views {
                    // The caller cannot know the ids made in this write, so a
                    // first view may name its properties and options.
                    let view = resolve_view_names(view, &schema);
                    check_view_def(&space_name, &draft, &view)?;
                    records = records.put_view(view_id, &view);
                }
                doc.edit_page(Some(&body), &patch, &records)
            }
        })?;
        Ok(note)
    }

    pub fn edit(&self, edit: EditPage, actor: &str) -> Result<NoteEditResult, BridgeError> {
        self.check_live(&edit.id)?;
        self.check_not_system(&edit.id)?;
        let modes = [edit.text.is_some(), !edit.edits.is_empty(), edit.append.is_some()];
        if modes.iter().filter(|mode| **mode).count() != 1 {
            return Err(invalid("send exactly one of text, edits or append"));
        }
        let mut warnings = Vec::new();
        let allow = edit.allow_deleting_children;
        let (note, seq, changed) = if let Some(text) = edit.text {
            let base_seq = edit.base_seq;
            self.write_with(&edit.id, actor, |db, live, current_seq| {
                let base = base_seq.unwrap_or(current_seq).min(current_seq);
                let before = live.body();
                let update = doc::merge_save(db, &edit.id, base, &text)?;
                if let Some(update) = &update {
                    live.apply(update)?;
                }
                // Judge the merged result: children added after `base` survive
                // the merge, so a stale save that never saw them deletes nothing.
                // An error here drops the uncommitted transaction.
                check_children_kept(&before, &live.body(), allow)?;
                Ok(update)
            })?
        } else if !edit.edits.is_empty() {
            self.write(&edit.id, actor, |live| {
                let current = live.body();
                let next = apply_edits(&current, &edit.edits)?;
                check_children_kept(&current, &next, allow)?;
                live.set_body(&next)
            })?
        } else {
            let append = edit.append.unwrap();
            if append.content.is_empty() {
                return Err(invalid("nothing to append"));
            }
            self.write(&edit.id, actor, |live| {
                let current = live.body();
                match append.position.as_str() {
                    "start" => {
                        let mut chunk = append.content.clone();
                        if !current.is_empty() && !chunk.ends_with('\n') {
                            chunk.push('\n');
                        }
                        live.insert_body(0, &chunk)
                    }
                    "end" => {
                        let mut chunk = String::new();
                        if !current.is_empty() && !current.ends_with('\n') {
                            chunk.push('\n');
                        }
                        chunk.push_str(&append.content);
                        live.insert_body(current.len(), &chunk)
                    }
                    other => Err(invalid(format!("append position must be start or end, not {other}"))),
                }
            })?
        };
        if !changed {
            warnings.push("nothing changed".to_owned());
        }
        Ok(NoteEditResult { note, seq, warnings })
    }

    pub fn update(&self, mut update: UpdateNote, actor: &str) -> Result<Note, BridgeError> {
        self.check_live(&update.id)?;
        self.check_not_system(&update.id)?;
        if let Some(kind) = &update.kind {
            check_kind(kind)?;
            // A database's schema and views live in a doc that only a
            // database page reads, so the kind changes in creation alone.
            if kind == "database" {
                return Err(invalid("a page is created as a database; it cannot become one later"));
            }
            let is_database: bool = self.with_db(|db| {
                Ok(db
                    .query_row("SELECT kind='database' FROM pages WHERE id=?1", params![update.id], |row| row.get(0))
                    .optional()?
                    .unwrap_or(false))
            })?;
            if is_database {
                return Err(invalid("a database keeps its kind"));
            }
        }
        if let Some(state) = &update.state {
            check_state(state)?;
        }
        if let Some(workspace) = &update.workspace_id {
            self.check_project(workspace)?;
        }
        if update.clear_parent && update.parent_id.is_some() {
            return Err(invalid("parentId and clearParent are mutually exclusive"));
        }
        // Cells go through `set_values`, the one path that checks them against
        // the row's database; an update that only sets cells writes nothing else.
        let values = std::mem::take(&mut update.values);
        let props_change = update.parent_id.is_some() || update.clear_parent || update.title.is_some() || update.kind.is_some() || update.tags.is_some()
            || update.workspace_id.is_some() || update.clear_workspace || update.state.is_some() || update.add_session_id.is_some();
        if !values.is_empty() && !props_change {
            return self.set_values(&update.id, &values, actor);
        }
        let _rename = update.title.is_some().then(|| self.rename_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let mut rename = None;
        let (note, _, _) = self.write_with(&update.id, actor, |db, live, _| {
            let props = live.props();
            if props.deleted_at.is_some() { return Err(invalid("note not found")); }
            // Validation and property changes share the database write lock, so
            // concurrent moves cannot both validate into a cycle.
            if let Some(parent) = &update.parent_id {
                validate_parent(db, &self.space, &update.id, parent)?;
            }
            let mut patch = PropsPatch::default();
            if update.clear_parent {
                patch = patch.clear(key::PARENT);
            } else if let Some(parent) = &update.parent_id {
                patch = patch.set(key::PARENT, parent);
            }
            if (update.clear_parent || update.parent_id.is_some()) && props.state.as_deref() == Some("inbox") && update.state.is_none() {
                patch = patch.set(key::STATE, "open");
            }
            if let Some(title) = update.title {
                let title = title.trim().to_owned();
                if let Some(old) = props.title.as_deref().filter(|old| !title.is_empty() && *old != title) {
                    // Only links resolved to this page move with it; per source,
                    // remember whether its `[[old]]` was one of them, since a
                    // duplicate title can send that link to another page.
                    let mut stmt = db.prepare("SELECT l.from_page_id, MAX(l.kind='wiki' AND l.target_key=?3) FROM page_links l JOIN pages p ON p.id=l.from_page_id AND p.space_id=l.space_id WHERE l.space_id=?1 AND l.to_page_id=?2 AND p.deleted_at IS NULL GROUP BY l.from_page_id")?;
                    let incoming = stmt.query_map(params![self.space, update.id, links::link_key(old)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                    rename = Some((old.to_owned(), title.clone(), incoming));
                }
                patch = patch.maybe(key::TITLE, (!title.is_empty()).then_some(title));
            }
            if let Some(kind) = update.kind {
                patch = patch.set(key::KIND, kind);
            }
            if let Some(tags) = update.tags {
                patch = patch.list(key::TAGS, &normalize_tags(&tags));
            }
            if update.clear_workspace {
                patch = patch.clear(key::WORKSPACE);
            } else if let Some(workspace) = update.workspace_id {
                patch = patch.set(key::WORKSPACE, workspace);
            }
            if let Some(state) = &update.state {
                patch = patch.set(key::STATE, state);
            }
            if let Some(session) = update.add_session_id.filter(|session| !session.trim().is_empty()) {
                let mut sessions = props.session_ids.clone();
                if !sessions.contains(&session) {
                    sessions.push(session);
                    // The chat may have taken time to start. Inspect the live
                    // state under the write lock so linking cannot reopen a
                    // task the person finished or archived in the meantime.
                    if update.state.is_none() && props.kind.as_deref() == Some("task") && matches!(props.state.as_deref(), Some("open" | "inbox")) {
                        patch = patch.set(key::STATE, "doing");
                    }
                }
                patch = patch.list(key::SESSIONS, &sessions);
            }
            // Cells land in the same write as the properties, checked against
            // the database the page belongs to once this write lands: a move and
            // its cells are one change, and a refused cell writes nothing.
            if values.is_empty() {
                return live.set_props(&patch);
            }
            let parent = if update.clear_parent { None } else { update.parent_id.clone().or_else(|| props.parent_id.clone()) };
            let database = match &parent {
                Some(parent) => database::read(db, parent)?,
                None => None,
            }
            .ok_or_else(|| invalid("only a row of a database has values"))?;
            if props.kind.as_deref() == Some("database") {
                return Err(invalid("a database has properties, not values"));
            }
            let space = props.space_id.clone().unwrap_or_else(|| self.space.clone());
            let records = cells_records(db, &space, &database, &live.values(), &values, (&update.id, parent.as_deref()))?;
            notices::clear(db, &space, &update.id, &values.iter().map(|(prop, _)| prop.clone()).collect::<Vec<_>>())?;
            live.edit_page(None, &patch, &records)
        })?;
        if let Some((old, new, incoming)) = rename {
            for (id, wiki) in incoming {
                self.write(&id, actor, |live| {
                    if live.props().deleted_at.is_some() { return Ok(None); }
                    let body = live.body();
                    let rewritten = links::rewrite_links_on_rename(&body, &update.id, &old, &new, wiki);
                    if rewritten == body { return Ok(None); }
                    live.edit(Some(&rewritten), &PropsPatch::default())
                })?;
            }
            // A self-link can change the renamed page again during propagation.
            return Ok(self.read(&update.id)?.note);
        }
        Ok(note)
    }

    /// A database's scope, schema and views, or an error when `id` is not a
    /// database page.
    pub fn database(&self, id: &str) -> Result<database::DatabaseDoc, BridgeError> {
        self.with_db(|db| database::read(db, id))?.ok_or_else(|| invalid("this page is not a database"))
    }

    /// Filter, sort, group and page one database's rows from the index.
    pub fn query_database(&self, id: &str, spec: &super::query::QuerySpec) -> Result<super::query::QueryResult, BridgeError> {
        self.with_db(|db| {
            let database = database::read(db, id)?.ok_or_else(|| invalid("this page is not a database"))?;
            super::query::run(db, &self.space, &database, spec)
        })
    }

    /// The space's system database: the page whose saved views are the Notes
    /// list's tabs.
    pub fn system_database(&self) -> Result<String, BridgeError> {
        let id: Option<Option<String>> = self.with_db(|db| {
            Ok(db
                .query_row("SELECT system_database_id FROM spaces WHERE id=?1", params![self.space], |row| row.get::<_, Option<String>>(0))
                .optional()?)
        })?;
        id.flatten().ok_or_else(|| invalid("this space has no system database"))
    }

    /// One system database per space, created on open and never twice.
    fn ensure_system_database(&self) -> Result<(), BridgeError> {
        let spaces: Vec<(String, Option<String>)> = self.with_db(|db| {
            let mut statement = db.prepare("SELECT id, system_database_id FROM spaces")?;
            let rows: Vec<(String, Option<String>)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
            Ok(rows)
        })?;
        for (space, existing) in spaces {
            let live = match &existing {
                Some(id) => self.with_db(|db| {
                    Ok(db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM pages WHERE id=?1 AND kind='database' AND deleted_at IS NULL)",
                        params![id],
                        |row| row.get::<_, bool>(0),
                    )?)
                })?,
                None => false,
            };
            if live {
                continue;
            }
            // A crash between creating the page and recording it leaves an
            // unclaimed system database behind; adopt it instead of growing a
            // second.
            let orphan: Option<String> = self.with_db(|db| {
                Ok(db
                    .query_row(
                        "SELECT id FROM pages
                         WHERE space_id=?1 AND kind='database' AND created_by=?2 AND deleted_at IS NULL
                           AND id NOT IN (SELECT system_database_id FROM spaces WHERE system_database_id IS NOT NULL)
                         ORDER BY created_at, id LIMIT 1",
                        params![space, SYSTEM],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?)
            })?;
            let (id, created) = match orphan {
                Some(orphan) => (orphan, false),
                None => (self.create_system_database(&space)?, true),
            };
            // Claim the space only if nobody else opened the same file first;
            // a loser drops the database it made so a space keeps exactly one.
            let claimed = self.with_db(|db| {
                Ok(db.execute(
                    "UPDATE spaces SET system_database_id=?2 WHERE id=?1 AND (system_database_id IS NULL OR system_database_id=?3)",
                    params![space, id, existing],
                )?)
            })?;
            if claimed == 0 && created {
                self.write(&id, SYSTEM, |live| live.set_props(&PropsPatch::default().set(key::DELETED_AT, now())))?;
            }
        }
        Ok(())
    }

    fn create_system_database(&self, space: &str) -> Result<String, BridgeError> {
        let id = new_id();
        let mut records = RecordsPatch::default().scope("space");
        for (view_id, view) in database::default_views() {
            records = records.put_view(&view_id, &view);
        }
        let patch = PropsPatch::default()
            .set(key::SPACE, space.to_owned())
            .set(key::TITLE, "Notes")
            .set(key::KIND, "database")
            .set(key::STATE, "open")
            .set(key::CREATED_BY, SYSTEM)
            .set(key::CREATED_AT, now());
        self.write_with(&id, SYSTEM, |_, live, _| live.edit_page(None, &patch, &records))?;
        Ok(id)
    }

    /// Set, or clear with `None`, one or more of a row's property values.
    ///
    /// The database is read inside the write's transaction, so a patch is
    /// validated against the schema as it stands at commit time, and a refused
    /// patch writes nothing.
    pub fn set_values(&self, id: &str, patch: &[(String, Option<Value>)], actor: &str) -> Result<Note, BridgeError> {
        self.check_live(id)?;
        if patch.is_empty() {
            return Err(invalid("no values to set"));
        }
        let (note, _, _) = self.write_with(id, actor, |db, live, _| {
            let props = live.props();
            if props.deleted_at.is_some() {
                return Err(invalid("note not found"));
            }
            if props.kind.as_deref() == Some("database") {
                return Err(invalid("a database has properties, not values"));
            }
            let parent = props.parent_id.clone().ok_or_else(|| invalid("only a row of a database has values"))?;
            let database = database::read(db, &parent)?.ok_or_else(|| invalid("only a row of a database has values"))?;
            let space = props.space_id.clone().unwrap_or_else(|| self.space.clone());
            let records = cells_records(db, &space, &database, &live.values(), patch, (id, Some(parent.as_str())))?;
            // A cell set here settles whatever the file said about it.
            notices::clear(db, &space, id, &patch.iter().map(|(prop, _)| prop.clone()).collect::<Vec<_>>())?;
            live.edit_records(&records)
        })?;
        Ok(note)
    }

    /// Change a database's schema. Retyping and deleting an option rewrite the
    /// affected rows' values as attributed writes, so history can undo them.
    ///
    /// Validation runs inside the write's transaction against the schema as it
    /// stands at commit time, so two callers cannot add the same name. The
    /// schema and the row writes are separate page commits: if a later step
    /// fails, the schema stays changed and the caller gets the error, and
    /// re-running the same ops is safe (a retype that is already done warns).
    pub fn update_schema(&self, id: &str, ops: &[SchemaOp], actor: &str) -> Result<SchemaUpdated, BridgeError> {
        self.check_live(id)?;
        if ops.is_empty() {
            return Err(invalid("no schema changes"));
        }
        // A batch may change a property once, unless every change to it is an
        // option edit: those fold into one def in order. Two structural changes
        // to one property, or an option edit beside one, could not be planned
        // against one shape.
        let mut seen: std::collections::BTreeMap<&str, bool> = std::collections::BTreeMap::new();
        for op in ops {
            let Some(prop) = op.property() else { continue };
            let structural = !op.is_option();
            match seen.get(prop.as_str()) {
                Some(true) => return Err(invalid("this batch changes the same property twice")),
                Some(false) if structural => return Err(invalid("this batch changes the same property twice")),
                Some(false) => {}
                None => {
                    seen.insert(prop, structural);
                }
            }
        }
        let mut plan: Option<SchemaPlan> = None;
        let (_, _, changed) = self.write_with(id, actor, |db, live, _| {
            let props = live.props();
            if props.deleted_at.is_some() {
                return Err(invalid("note not found"));
            }
            if props.kind.as_deref() != Some("database") {
                return Err(invalid("this page is not a database"));
            }
            let before = live.schema();
            let mut schema = before.clone();
            let mut next = SchemaPlan { rewrite: false, ..Default::default() };
            for op in ops {
                match op {
                    SchemaOp::Add(new) => {
                        let def = build_property(db, &schema, new)?;
                        let name = def.name.clone();
                        let prop = new_id();
                        schema.insert(prop.clone(), def.clone());
                        next.updated.added.push((prop.clone(), name.clone()));
                        // A name a delete retired starts empty: the old lines
                        // are foreign keys, not values to restore silently.
                        if !live.retired().keys().any(|old| old.to_lowercase() == name.to_lowercase()) {
                            next.imported.push((prop, def));
                        }
                    }
                    SchemaOp::Rename { prop, name } => {
                        let old = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        let name = database::check_name(&schema, name, Some(prop))?;
                        if old.name != name {
                            next.retire.push((old.name.clone(), prop.clone()));
                            next.rewrite = true;
                            let def = PropertyDef { name, ..old };
                            schema.insert(prop.clone(), def.clone());
                            next.imported.push((prop.clone(), def));
                        }
                    }
                    SchemaOp::Delete { prop } => {
                        let old = schema.remove(prop).ok_or_else(|| no_property(prop))?;
                        next.retire.push((old.name, prop.clone()));
                        next.rewrite = true;
                    }
                    SchemaOp::Retype { prop, to } => {
                        let old = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        let kind = PropertyType::parse(to)
                            .ok_or_else(|| invalid(format!("type must be one of {}", database::PROPERTY_TYPES.join(", "))))?;
                        if kind == old.kind {
                            next.updated.warnings.push(format!("{} is already {}", old.name, kind.as_str()));
                            // A retype that failed part way leaves values of the
                            // old type behind. Re-running repairs them instead of
                            // warning past values nothing can render.
                            let values = database::row_values(db, id, prop)?;
                            for (row, value) in values {
                                if database::validate_value(&old, &value).is_err() {
                                    next.retypes.push((row, prop.clone(), old.clone(), old.clone(), value));
                                }
                            }
                            continue;
                        }
                        if kind == PropertyType::Relation {
                            return Err(invalid("a relation needs the database it points at; add one instead of retyping"));
                        }
                        let mut converted = PropertyDef { kind, ..old.clone() };
                        // The fields that belong to one type do not carry over.
                        converted.relation = None;
                        converted.date_format = if kind == PropertyType::Date { old.date_format.clone() } else { None };
                        if kind == PropertyType::Status {
                            for option in &mut converted.options {
                                option.group.get_or_insert_with(|| "todo".to_owned());
                            }
                        } else {
                            for option in &mut converted.options {
                                option.group = None;
                            }
                        }
                        // Every row's current value, so a text -> select retype
                        // can create all its options before any row is written.
                        let values = database::row_values(db, id, prop)?;
                        for (_, value) in &values {
                            database::convert(&old, &mut converted, value);
                        }
                        for (row, value) in values {
                            next.retypes.push((row, prop.clone(), old.clone(), converted.clone(), value));
                        }
                        schema.insert(prop.clone(), converted);
                    }
                    SchemaOp::AddOption { prop, name, color, group } => {
                        let mut def = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        if !def.kind.options() {
                            return Err(invalid(format!("{} does not have options", def.name)));
                        }
                        let name = database::check_option_name(&def, name, None)?;
                        let group = option_group(def.kind, group)?;
                        def.options.push(OptionDef { id: new_id(), name, color: color.clone(), group });
                        schema.insert(prop.clone(), def);
                    }
                    SchemaOp::RenameOption { prop, option, name } => {
                        let mut def = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        let name = database::check_option_name(&def, name, Some(option))?;
                        let existing = def.options.iter_mut().find(|entry| &entry.id == option).ok_or_else(|| invalid(format!("{} has no option {option}", def.name)))?;
                        if existing.name != name {
                            existing.name = name;
                            next.rewrite = true;
                        }
                        schema.insert(prop.clone(), def);
                    }
                    SchemaOp::DeleteOption { prop, option } => {
                        let mut def = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        let position = def.options.iter().position(|entry| &entry.id == option).ok_or_else(|| invalid(format!("{} has no option {option}", def.name)))?;
                        def.options.remove(position);
                        next.dropped.push((prop.clone(), option.clone()));
                        schema.insert(prop.clone(), def);
                    }
                    SchemaOp::SetOptions { prop, options, base } => {
                        let def = schema.get(prop).ok_or_else(|| no_property(prop))?.clone();
                        if !def.kind.options() {
                            return Err(invalid(format!("{} does not have options", def.name)));
                        }
                        if base.as_ref().is_some_and(|base| !def.options.iter().map(|option| &option.id).eq(base.iter())) {
                            return Err(invalid(format!("the options of {} changed since they were read; read them again", def.name)));
                        }
                        let mut built = PropertyDef { options: Vec::new(), ..def.clone() };
                        for input in options {
                            let existing = match &input.id {
                                Some(id) => Some(def.option(id).ok_or_else(|| invalid(format!("{} has no option {id}", def.name)))?.clone()),
                                None => None,
                            };
                            if let Some(existing) = &existing {
                                if built.option(&existing.id).is_some() {
                                    return Err(invalid(format!("{} lists option {} twice", def.name, existing.id)));
                                }
                            }
                            let name = database::check_option_name(&built, &input.name, None)?;
                            let group = match (&input.group, &existing) {
                                (None, Some(existing)) if def.kind == PropertyType::Status => existing.group.clone(),
                                (group, _) => option_group(def.kind, group)?,
                            };
                            let color = input.color.clone().or_else(|| existing.as_ref().and_then(|existing| existing.color.clone()));
                            if existing.as_ref().is_some_and(|existing| existing.name != name) {
                                next.rewrite = true;
                            }
                            let id = existing.map(|existing| existing.id).unwrap_or_else(new_id);
                            built.options.push(OptionDef { id, name, color, group });
                        }
                        // Every option a row still names that the new list lacks: the
                        // ones this call drops, and any an interrupted earlier
                        // cleanup left behind, so a retry finishes it.
                        let mut stale: std::collections::BTreeSet<String> = def.options.iter().filter(|option| built.option(&option.id).is_none()).map(|option| option.id.clone()).collect();
                        for (_, value) in database::row_values(db, id, prop)? {
                            let named: Vec<String> = match value {
                                Value::String(option) => vec![option],
                                Value::Array(items) => items.into_iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
                                _ => Vec::new(),
                            };
                            stale.extend(named.into_iter().filter(|option| built.option(option).is_none()));
                        }
                        next.dropped.extend(stale.into_iter().map(|option| (prop.clone(), option)));
                        schema.insert(prop.clone(), built);
                    }
                    SchemaOp::Reorder { props } => {
                        let named: std::collections::BTreeSet<&String> = props.iter().collect();
                        if named.len() != props.len() || named.len() != schema.len() || props.iter().any(|prop| !schema.contains_key(prop)) {
                            return Err(invalid(format!("reorder names every property once: {}", schema.keys().cloned().collect::<Vec<_>>().join(", "))));
                        }
                        for (order, prop) in props.iter().enumerate() {
                            if let Some(def) = schema.get_mut(prop) {
                                if def.order != order as i64 {
                                    def.order = order as i64;
                                    next.rewrite = true;
                                }
                            }
                        }
                    }
                }
            }

            let mut records = RecordsPatch::default();
            for (prop, def) in &schema {
                if before.get(prop) != Some(def) {
                    records = records.put_schema(prop, def);
                }
            }
            for prop in before.keys().filter(|prop| !schema.contains_key(*prop)) {
                records = records.remove_schema(prop);
            }
            for (old, prop) in &next.retire {
                records = records.retire(old, prop);
            }
            // Every saved view follows the new schema in this same write, so a
            // view is always queryable: what names a removed property or option,
            // or asks of a property what its new type cannot do, goes.
            let draft = database::DatabaseDoc { id: id.to_owned(), scope: live.scope().unwrap_or_else(|| "children".to_owned()), schema: schema.clone(), views: Default::default(), retired: Default::default() };
            for (view_id, view) in live.views() {
                let pruned = prune_view(&self.space, &draft, &view);
                if pruned != view {
                    records = records.put_view(&view_id, &pruned);
                }
            }
            // A name that is live again is no longer retired.
            let live_names: std::collections::BTreeSet<String> = schema.values().map(|def| def.name.to_lowercase()).collect();
            for name in live.retired().keys() {
                if live_names.contains(&name.to_lowercase()) {
                    records = records.restore_name(name);
                }
            }
            if records.is_empty() {
                // A same-type retype repairs stranded values without a schema
                // change, so the plan still has work to do.
                plan = Some(next);
                return Ok(None);
            }
            plan = Some(next);
            live.edit_records(&records)
        })?;
        let Some(plan) = plan else { return Ok(SchemaUpdated::default()) };
        if changed {
            // Lines a row's file already had for a newly added or renamed
            // property are the person's existing data; they come in before the
            // next projection would drop them.
            for (prop, def) in &plan.imported {
                self.import_file_values(id, prop, def, actor)?;
            }
        }
        // Values follow the schema in separate attributed writes, one per row.
        // These also run when only a repair is left to do.
        for (row, prop, old, next, value) in plan.retypes {
            self.convert_row(&row, &prop, &old, &next, &value, actor)?;
        }
        for (prop, option) in plan.dropped {
            self.drop_option(id, &prop, &option, actor)?;
        }
        if changed {
            self.with_db(|db| database::reindex_database(db, id))?;
            if plan.rewrite {
                // The rows' docs did not change, so their files are stale
                // without being behind on a sequence. The maintenance pass
                // rewrites them with the retired names in hand, off this
                // request's path.
                self.with_db(|db| Ok(db.execute("UPDATE pages SET projected_seq=NULL WHERE parent_id=?1 AND deleted_at IS NULL", params![id])?))?;
            }
        }
        Ok(plan.updated)
    }

    /// Import lines a row's file already had for a property just added or
    /// renamed, so an existing foreign key or old name becomes the cell.
    fn import_file_values(&self, database_id: &str, prop: &str, def: &PropertyDef, actor: &str) -> Result<(), BridgeError> {
        let (space, rows): (String, Vec<(String, Option<String>)>) = self.with_db(|db| {
            let space = db.query_row("SELECT space_id FROM pages WHERE id=?1", params![database_id], |row| row.get(0))?;
            let mut statement = db.prepare("SELECT id, path FROM pages WHERE parent_id=?1 AND deleted_at IS NULL")?;
            let rows: Vec<(String, Option<String>)> = statement.query_map(params![database_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
            Ok((space, rows))
        })?;
        for (row, path) in rows {
            let Some(path) = path else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let (front, _) = super::markdown::parse(&text);
            let key = def.name.to_lowercase();
            let Some(line) = front.unknown.iter().find(|line| line.split_once(':').is_some_and(|(name, _)| name.trim().to_lowercase() == key)) else { continue };
            let Some((_, raw)) = line.split_once(':') else { continue };
            match self.with_db(|db| Ok(database::parse_value(db, &space, def, raw)))? {
                Some(value) => {
                    self.set_values(&row, &[(prop.to_owned(), Some(value))], actor)?;
                }
                None => eprintln!("[notes] left {raw} of {row} unimported: it is not a valid {}", def.kind.as_str()),
            }
        }
        Ok(())
    }

    /// A property's value on every row, with the row id.
    /// One row's converted value, written only when it changed since the scan.
    fn convert_row(&self, row: &str, prop: &str, old: &PropertyDef, next: &PropertyDef, value: &Value, actor: &str) -> Result<(), BridgeError> {
        let mut scratch = next.clone();
        // A repair runs with `old == next`; the value's own shape is the type
        // it came from, so `5` in a text column still becomes "5".
        let source = if old.kind == next.kind {
            database::infer_type(value).map(|kind| PropertyDef { kind, ..old.clone() }).unwrap_or_else(|| old.clone())
        } else {
            old.clone()
        };
        let converted = database::convert(&source, &mut scratch, value);
        if converted.as_ref() == Some(value) {
            return Ok(());
        }
        let converted = converted.filter(|converted| database::validate_value(next, converted).is_ok());
        self.write(row, actor, |live| {
            if live.values().get(prop) != Some(value) {
                return Ok(None);
            }
            let mut records = RecordsPatch::default().put_value(prop, converted.as_ref());
            if converted.is_none() {
                // Keep the cleared value so a projection that has to wait for
                // an outside edit still knows which line is stale.
                records = records.clear_value(prop, value);
            }
            live.edit_records(&records)
        })?;
        Ok(())
    }

    /// Remove a deleted option from every row that used it, one attributed
    /// write per row.
    fn drop_option(&self, database_id: &str, prop: &str, option: &str, actor: &str) -> Result<(), BridgeError> {
        for (row, value) in self.with_db(|db| database::row_values(db, database_id, prop))? {
            let next = match &value {
                Value::String(id) if id == option => None,
                Value::Array(items) => {
                    let kept: Vec<Value> = items.iter().filter(|item| item.as_str() != Some(option)).cloned().collect();
                    (!kept.is_empty()).then_some(Value::Array(kept))
                }
                _ => continue,
            };
            self.write(&row, actor, |live| {
                if live.values().get(prop) != Some(&value) {
                    return Ok(None);
                }
                let mut records = RecordsPatch::default().put_value(prop, next.as_ref());
                if next.is_none() {
                    records = records.clear_value(prop, &value);
                }
                live.edit_records(&records)
            })?;
        }
        Ok(())
    }

    /// Save one view of a database, replacing it when `view_id` names one.
    pub fn put_view(&self, id: &str, view_id: Option<&str>, view: ViewDef, actor: &str) -> Result<String, BridgeError> {
        self.check_live(id)?;
        let database = self.database(id)?;
        let view_id = view_id.map(str::to_owned).unwrap_or_else(new_id);
        let mut view = view;
        view.name = view.name.trim().to_owned();
        check_view_def(&self.space, &database, &view)?;
        // The count is checked against the live doc inside the write, so two
        // saves at once cannot pass the same check.
        self.write(id, actor, |live| {
            let views = live.views();
            if !views.contains_key(&view_id) && views.len() >= MAX_VIEWS {
                return Err(invalid(format!("a database holds at most {MAX_VIEWS} views")));
            }
            live.edit_records(&RecordsPatch::default().put_view(&view_id, &view))
        })?;
        Ok(view_id)
    }

    /// Make a database with its first properties and views, in the one write
    /// that makes the page: a refused property or view makes nothing. With no
    /// properties it gets a Status with To do, In progress and Done; with no
    /// views, one table.
    pub fn create_database(&self, create: CreateDatabase, actor: &str) -> Result<Note, BridgeError> {
        if create.views.len() > MAX_VIEWS {
            return Err(invalid(format!("a database holds at most {MAX_VIEWS} views")));
        }
        let properties = if create.properties.is_empty() {
            let option = |name: &str, group: &str| NewOption { name: name.into(), group: Some(group.into()), ..Default::default() };
            vec![NewProperty { name: "Status".into(), kind: "status".into(), options: vec![option("To do", "todo"), option("In progress", "inProgress"), option("Done", "done")], ..Default::default() }]
        } else {
            create.properties
        };
        self.create(
            CreatePage {
                title: create.title,
                content: create.content,
                tags: create.tags,
                parent_id: create.parent_id,
                workspace_id: create.workspace_id,
                kind: Some("database".into()),
                properties,
                views: create.views,
                ..Default::default()
            },
            actor,
        )
    }

    pub fn delete_view(&self, id: &str, view_id: &str, actor: &str) -> Result<(), BridgeError> {
        self.check_live(id)?;
        self.database(id)?;
        // Checked inside the write: two deletes at once cannot both see two
        // views and leave none.
        self.write(id, actor, |live| {
            let views = live.views();
            if !views.contains_key(view_id) {
                return Err(invalid("no such view"));
            }
            if views.len() == 1 {
                return Err(invalid("a database keeps at least one view"));
            }
            live.edit_records(&RecordsPatch::default().remove_view(view_id))
        })?;
        Ok(())
    }

    /// Tombstone a page and its tasks.
    pub fn delete(&self, id: &str, actor: &str) -> Result<(), BridgeError> {
        self.check_live(id)?;
        self.check_not_system(id)?;
        let tasks: Vec<String> = self.with_db(|db| {
            let mut statement = db.prepare("SELECT id FROM pages WHERE parent_id=?1 AND kind='task' AND deleted_at IS NULL")?;
            let ids = statement.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })?;
        let stamp = now();
        // Each task is rechecked inside its own write: one moved elsewhere
        // since the snapshot keeps its new home.
        let tombstone_task = |task: &str| {
            self.write(task, actor, |live| {
                let props = live.props();
                if props.deleted_at.is_some() || props.parent_id.as_deref() != Some(id) { return Ok(None); }
                live.set_props(&PropsPatch::default().set(key::DELETED_AT, stamp.clone()))
            })
        };
        for task in &tasks {
            tombstone_task(task)?;
        }
        self.write(id, actor, |live| live.set_props(&PropsPatch::default().set(key::DELETED_AT, stamp.clone())))?;
        self.with_db(|db| notices::clear_page(db, id))?;
        // A task moved in before the parent's tombstone still goes with it; the
        // tombstone now refuses further moves.
        let late: Vec<String> = self.with_db(|db| {
            let mut statement = db.prepare("SELECT id FROM pages WHERE parent_id=?1 AND kind='task' AND deleted_at IS NULL")?;
            let ids = statement.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })?;
        for task in &late {
            tombstone_task(task)?;
        }
        Ok(())
    }

    pub fn read(&self, id: &str) -> Result<NotePage, BridgeError> {
        self.with_db(|db| {
            let (note, body, hash) = db
                .query_row(&format!("SELECT {NOTE_COLUMNS}, body, body_hash FROM pages WHERE id=?1 AND deleted_at IS NULL"), params![id], |row| {
                    Ok((note_from_row(row)?, row.get::<_, String>(19)?, row.get::<_, String>(20)?))
                })
                .optional()?
                .ok_or_else(|| invalid("note not found"))?;
            let structured = read_structured(db, id, &hash)?;
            let mut statement = db.prepare(&format!(
                "SELECT {NOTE_COLUMNS} FROM pages WHERE parent_id=?1 AND kind='task' AND deleted_at IS NULL ORDER BY created_at, id"
            ))?;
            let tasks = statement.query_map(params![id], note_from_row)?.collect::<Result<_, _>>()?;
            let backlinks = links::read_backlinks(db, &note.space_id, id)?;
            // A database carries its records; a row its cells, only those its
            // database still has a property for.
            let database = if note.kind == "database" { database::read(db, id)?.map(|doc| super::wire::database(&doc)) } else { None };
            let database_id = database::database_of(db, id)?;
            let values = match &database_id {
                Some(parent) => {
                    let schema = database::read(db, parent)?.map(|doc| doc.schema).unwrap_or_default();
                    let mut values = doc::load_page(db, id, None)?.0.values();
                    values.retain(|prop, _| schema.contains_key(prop));
                    Some(values)
                }
                None => None,
            };
            // Only a row can hold notices, so no other page pays for the read.
            let notices = if database_id.is_some() { notices::read(db, &note.space_id, id)? } else { Vec::new() };
            Ok(NotePage { seq: note.seq, note, markdown: body, structured, tasks, backlinks, database, database_id, values, notices })
        })
    }

    pub fn list(&self, filter: &ListFilter) -> Result<Vec<Note>, BridgeError> {
        self.with_db(|db| {
            let query = filter.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
            let fts = query.and_then(links::fts_query);
            let mut args: Vec<rusqlite::types::Value> = vec![self.space.clone().into()];
            // FTS5 weights include the two UNINDEXED identity columns.
            let mut sql = if let Some(fts) = &fts {
                args.push(fts.clone().into());
                format!("SELECT {NOTE_COLUMNS} FROM pages JOIN (SELECT page_id, bm25(pages_fts, 0.0, 0.0, 5.0, 1.0, 2.0) AS rank FROM pages_fts WHERE pages_fts MATCH ?2) search ON search.page_id=pages.id WHERE deleted_at IS NULL AND space_id=?1")
            } else {
                format!("SELECT {NOTE_COLUMNS} FROM pages WHERE deleted_at IS NULL AND space_id=?1")
            };
            let mut bind = |sql: &mut String, clause: &str, value: String| {
                args.push(value.into());
                sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
            };
            if !filter.include_archived && filter.state.as_deref() != Some("archived") {
                sql.push_str(" AND state<>'archived'");
            }
            // The system database is a page like any other, but the Notes list
            // shows its views instead of it.
            sql.push_str(" AND id NOT IN (SELECT system_database_id FROM spaces WHERE system_database_id IS NOT NULL)");
            if let Some(query) = query.filter(|_| fts.is_none()) {
                let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                bind(&mut sql, " AND (title LIKE ? ESCAPE '\\' OR body LIKE ? ESCAPE '\\')", pattern);
            }
            for tag in normalize_tags(&filter.tags) {
                bind(&mut sql, " AND EXISTS (SELECT 1 FROM json_each(pages.tags) WHERE value=?)", tag);
            }
            if let Some(kind) = &filter.kind {
                bind(&mut sql, " AND kind=?", kind.clone());
            }
            if let Some(state) = &filter.state {
                bind(&mut sql, " AND state=?", state.clone());
            }
            if let Some(parent) = &filter.parent_id {
                bind(&mut sql, " AND parent_id=?", parent.clone());
            }
            sql.push_str(&format!(" ORDER BY {}updated_at DESC, id DESC LIMIT {}", if fts.is_some() { "search.rank, " } else { "" }, filter.limit.unwrap_or(500).min(2000)));
            let mut statement = db.prepare(&sql)?;
            let notes = statement.query_map(rusqlite::params_from_iter(args.iter()), note_from_row)?.collect::<Result<_, _>>()?;
            Ok(notes)
        })
    }

    /// Drop the index and rebuild every page from `page_updates`.
    pub fn rebuild_index(&self) -> Result<usize, BridgeError> {
        let db = self.db.lock().unwrap();
        let transaction = db.unchecked_transaction()?;
        let paths: Vec<(String, Option<String>, Option<String>, Option<i64>)> = {
            let mut statement = transaction.prepare("SELECT id, path, content_hash, projected_seq FROM pages")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?.collect::<Result<_, _>>()?;
            rows
        };
        transaction.execute("DELETE FROM pages_fts", [])?;
        transaction.execute("DELETE FROM page_links", [])?;
        // Not in the log, so not rebuilt: the next import of a file that still
        // does not read records them again.
        transaction.execute("DELETE FROM page_notices", [])?;
        transaction.execute("DELETE FROM pages", [])?;
        let files = projector::find_files(&self.root());
        let pages: Vec<String> = {
            let mut statement = transaction.prepare("SELECT DISTINCT page_id FROM page_updates ORDER BY page_id")?;
            let rows = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
            rows
        };
        for page in &pages {
            let (live, seq) = doc::load_page(&transaction, page, None)?;
            let (actor, hlc): (String, String) = transaction.query_row(
                "SELECT actor, hlc FROM page_updates WHERE page_id=?1 ORDER BY seq DESC LIMIT 1",
                params![page],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            upsert_index(&transaction, page, &live, seq, &actor, &stamp_time(&hlc), &self.space, false)?;
            if let Some((_, path, hash, projected)) = paths.iter().find(|(id, _, _, _)| id == page) {
                transaction.execute("UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1", params![page, path, hash, projected])?;
            } else if let Some((path, hash)) = files.get(page) {
                // The index was lost with its paths; the files still name their pages.
                transaction.execute(
                    "UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1",
                    params![page, path.to_string_lossy(), hash, seq],
                )?;
            }
        }
        let spaces: Vec<String> = {
            let mut statement = transaction.prepare("SELECT DISTINCT space_id FROM pages")?;
            let rows = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
            rows
        };
        for space in spaces {
            links::resolve_space(&transaction, &space)?;
        }
        // The values index is rebuilt only once every page exists, so a row
        // finds its database's schema whatever order the log replays in.
        transaction.execute("DELETE FROM database_props", [])?;
        transaction.execute("DELETE FROM page_values", [])?;
        let databases: Vec<String> = {
            let mut statement = transaction.prepare("SELECT id FROM pages WHERE kind='database' AND deleted_at IS NULL")?;
            let ids: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
            ids
        };
        for database in databases {
            database::reindex_database(&transaction, &database)?;
        }
        transaction.commit()?;
        Ok(pages.len())
    }

    fn check_project(&self, workspace: &str) -> Result<(), BridgeError> {
        if (self.project_exists)(workspace) {
            Ok(())
        } else {
            Err(invalid(format!("no project with id {workspace}")))
        }
    }

    fn check_live(&self, id: &str) -> Result<(), BridgeError> {
        let live: bool = self.with_db(|db| {
            Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM pages WHERE id=?1 AND deleted_at IS NULL)", params![id], |row| row.get(0))?)
        })?;
        if live {
            Ok(())
        } else {
            Err(invalid("note not found"))
        }
    }

    /// The system database is the Notes list's own record: Bridge owns its
    /// identity and lifetime, a person owns its views.
    fn check_not_system(&self, id: &str) -> Result<(), BridgeError> {
        let system: bool = self.with_db(|db| {
            Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM spaces WHERE system_database_id=?1)", params![id], |row| row.get(0))?)
        })?;
        if system {
            Err(invalid("the Notes database is managed by Bridge"))
        } else {
            Ok(())
        }
    }

    /// The one write path. `change` makes at most one Yjs update on the live doc.
    pub(crate) fn write(
        &self,
        page_id: &str,
        actor: &str,
        change: impl FnOnce(&PageDoc) -> Result<Option<Vec<u8>>, BridgeError>,
    ) -> Result<(Note, i64, bool), BridgeError> {
        self.write_with(page_id, actor, |_, live, _| change(live))
    }

    fn write_with(
        &self,
        page_id: &str,
        actor: &str,
        change: impl FnOnce(&Connection, &PageDoc, i64) -> Result<Option<Vec<u8>>, BridgeError>,
    ) -> Result<(Note, i64, bool), BridgeError> {
        let (note, seq, changed) = {
            let db = self.db.lock().unwrap();
            let transaction = db.unchecked_transaction()?;
            let (live, current_seq) = doc::load_page(&transaction, page_id, None)?;
            let update = change(&transaction, &live, current_seq)?;
            let (seq, changed) = match update {
                Some(bytes) => {
                    let hlc = self.clock.lock().unwrap().now();
                    let seq = doc::append_update(&transaction, page_id, &bytes, actor, &hlc, &self.device)?;
                    doc::maybe_snapshot(&transaction, page_id, seq, &live)?;
                    upsert_index(&transaction, page_id, &live, seq, actor, &now(), &self.space, true)?;
                    (seq, true)
                }
                None => (current_seq, false),
            };
            transaction.commit()?;
            let note = db.query_row(&format!("SELECT {NOTE_COLUMNS} FROM pages WHERE id=?1"), params![page_id], note_from_row)?;
            (note, seq, changed)
        };
        if !changed {
            return Ok((note, seq, false));
        }
        let note = self.project(page_id)?.unwrap_or(note);
        let listener = self.listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(&NotePageChanged { page_id: page_id.to_owned(), actor: actor.to_owned(), seq });
        }
        Ok((note, seq, true))
    }

    /// Mirror the newest committed version of a page, if it is not mirrored
    /// yet. Serialized, and it reads the row after taking the lock, so an
    /// earlier snapshot can never overwrite a later one or mistake Bridge's
    /// own previous write for an outside edit.
    pub(crate) fn project(&self, page_id: &str) -> Result<Option<Note>, BridgeError> {
        let _projection = self.projection_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let row = self.with_db(|db| {
            Ok(db
                .query_row(&format!("SELECT {NOTE_COLUMNS}, body, content_hash, projected_seq FROM pages WHERE id=?1"), params![page_id], |row| {
                    Ok((
                        note_from_row(row)?,
                        row.get::<_, Option<String>>(18)?.is_some(),
                        row.get::<_, String>(19)?,
                        row.get::<_, Option<String>>(20)?,
                        row.get::<_, Option<i64>>(21)?,
                    ))
                })
                .optional()?)
        })?;
        let Some((note, deleted, body, hash, projected)) = row else { return Ok(None) };
        // The system database is a record, not a page: it has no file.
        if note.created_by == SYSTEM && note.kind == "database" {
            self.with_db(|db| Ok(db.execute("UPDATE pages SET projected_seq=?2, path=NULL, content_hash=NULL WHERE id=?1", params![note.id, note.seq])?))?;
            return Ok(Some(note));
        }
        if projected.is_some_and(|projected| projected >= note.seq) {
            return Ok(Some(note));
        }
        let records = self.page_records(&note)?;
        let input = projector::MirrorInput {
            note: &note,
            body: &body,
            deleted,
            previous_path: note.path.as_deref(),
            previous_hash: hash.as_deref(),
            database: records.database.as_deref(),
            properties: &records.properties,
            owned: &records.owned,
            retired: &records.retired,
            drop: &records.drop,
        };
        let root = self.root();
        let Some(mirrored) = projector::mirror(&root, &input) else {
            return Ok(Some(note));
        };
        if !deleted {
            assets::mirror_referenced(self, &body, &root);
        }
        self.with_db(|db| {
            db.execute(
                "UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1",
                params![note.id, mirrored.path, mirrored.hash, note.seq],
            )?;
            Ok(())
        })?;
        Ok(Some(Note { path: mirrored.path, ..note }))
    }

    /// The frontmatter a page's file carries beyond the known keys: a database's
    /// `{scope, schema, views}` JSON, or a row's values by property name.
    fn page_records(&self, note: &Note) -> Result<MirroredRecords, BridgeError> {
        self.with_db(|db| {
            let live = doc::load_page(db, &note.id, None)?.0;
            if note.kind == "database" {
                let value = serde_json::json!({
                    "scope": live.scope().unwrap_or_else(|| "children".to_owned()),
                    "schema": live.schema(),
                    "views": live.views(),
                });
                return Ok(MirroredRecords { database: Some(serde_json::to_string(&value).unwrap_or_default()), ..Default::default() });
            }
            let Some(parent) = note.parent_id.as_deref() else { return Ok(MirroredRecords::default()) };
            let Some(database) = database::read(db, parent)? else { return Ok(MirroredRecords::default()) };
            let values = live.values();
            let mut records = MirroredRecords::default();
            let mut rendered: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
            for (prop, def) in database.ordered() {
                records.owned.push(def.name.clone());
                let Some(value) = values.get(prop) else { continue };
                if let Some(text) = database::render_value(db, &note.space_id, def, value) {
                    rendered.insert(prop.clone(), text.clone());
                    records.properties.push((def.name.clone(), text));
                }
            }
            for (old, prop) in &database.retired {
                records.retired.push((old.clone(), rendered.get(prop).cloned()));
            }
            for (prop, stale) in live.cleared() {
                let Some(def) = database.schema.get(&prop) else { continue };
                records.drop.push((def.name.clone(), database::render_value(db, &note.space_id, def, &stale)));
            }
            Ok(records)
        })
    }

    /// Announce a change made beside the page rather than to it (a stored
    /// structured view), at the page's current sequence.
    pub fn announce(&self, page_id: &str, actor: &str) -> Result<(), BridgeError> {
        let seq: i64 = self.with_db(|db| Ok(db.query_row("SELECT seq FROM pages WHERE id=?1", params![page_id], |row| row.get(0))?))?;
        let listener = self.listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(&NotePageChanged { page_id: page_id.to_owned(), actor: actor.to_owned(), seq });
        }
        Ok(())
    }

    /// Exclusive right to import outside file edits; see `projector::import_changes`.
    pub(crate) fn import_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.import_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The device this service stamps changes with.
    pub fn device(&self) -> &str {
        &self.device
    }
}

pub(crate) const NOTE_COLUMNS: &str = "id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt, \
     substr(body, 1, 280), path, seq, created_by, last_actor, created_at, updated_at, deleted_at";

/// The frontmatter a page's file carries beyond the keys every page has.
#[derive(Debug, Clone, Default)]
struct MirroredRecords {
    /// A database page's `{scope, schema, views}` JSON.
    database: Option<String>,
    /// A row's values, by property name, in schema order.
    properties: Vec<(String, String)>,
    /// Every property name the row's schema owns.
    owned: Vec<String>,
    /// Old names a rename or a delete retired, with the value the document
    /// writes under the property's current name (`None` when it is gone).
    retired: Vec<(String, Option<String>)>,
    /// Cleared cells whose stale line has not left the file yet, with the
    /// rendered value that line must match to be dropped.
    drop: Vec<(String, Option<String>)>,
}

pub(crate) fn note_from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    let list = |index: usize| -> rusqlite::Result<Vec<String>> {
        Ok(serde_json::from_str(&row.get::<_, String>(index)?).unwrap_or_default())
    };
    let preview: String = row.get(11)?;
    Ok(Note {
        id: row.get(0)?,
        space_id: row.get(1)?,
        parent_id: row.get(2)?,
        source_id: row.get(3)?,
        title: row.get(4)?,
        kind: row.get(5)?,
        tags: list(6)?,
        workspace_id: row.get(7)?,
        state: row.get(8)?,
        session_ids: list(9)?,
        source_excerpt: row.get(10)?,
        preview: preview.chars().take(PREVIEW_CHARS).collect(),
        path: row.get(12)?,
        seq: row.get(13)?,
        created_by: row.get(14)?,
        last_actor: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// `resolve` is false during a rebuild, which resolves each space once at the end.
#[allow(clippy::too_many_arguments)]
fn upsert_index(db: &Connection, page_id: &str, live: &PageDoc, seq: i64, actor: &str, updated_at: &str, default_space: &str, resolve: bool) -> Result<(), BridgeError> {
    let props = live.props();
    let body = live.body();
    let space = props.space_id.clone().unwrap_or_else(|| default_space.to_owned());
    let system = props.created_by.as_deref() == Some(SYSTEM) && props.kind.as_deref() == Some("database");
    let created_at = props.created_at.clone().unwrap_or_else(|| updated_at.to_owned());
    db.execute(
        "INSERT INTO pages(id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt,
                           body, body_hash, seq, created_by, last_actor, created_at, updated_at, deleted_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
         ON CONFLICT(id) DO UPDATE SET
            space_id=excluded.space_id, parent_id=excluded.parent_id, source_id=excluded.source_id, title=excluded.title,
            kind=excluded.kind, tags=excluded.tags, workspace_id=excluded.workspace_id, state=excluded.state,
            session_ids=excluded.session_ids, source_excerpt=excluded.source_excerpt, body=excluded.body,
            body_hash=excluded.body_hash, seq=excluded.seq, last_actor=excluded.last_actor,
            updated_at=excluded.updated_at, deleted_at=excluded.deleted_at",
        params![
            page_id,
            space,
            props.parent_id,
            props.source_id,
            props.title,
            props.kind.unwrap_or_else(|| "note".into()),
            serde_json::to_string(&props.tags).unwrap_or_else(|_| "[]".into()),
            props.workspace_id,
            props.state.unwrap_or_else(|| "open".into()),
            serde_json::to_string(&props.session_ids).unwrap_or_else(|_| "[]".into()),
            props.source_excerpt,
            body,
            body_hash(&body),
            seq,
            props.created_by.unwrap_or_else(|| actor.to_owned()),
            actor,
            created_at,
            updated_at,
            props.deleted_at,
        ],
    )?;
    links::sync_fts(db, page_id)?;
    if resolve {
        links::sync_page_links(db, page_id)?;
        // A database's properties, or a row's values, follow every write.
        database::reindex_page(db, &space, page_id, live)?;
    } else {
        links::write_outbound(db, page_id)?;
    }
    // The system database is a record, not a page: it resolves no link and
    // answers no search.
    if system {
        db.execute("UPDATE pages SET link_key=NULL WHERE id=?1", params![page_id])?;
        db.execute("DELETE FROM pages_fts WHERE rowid=(SELECT fts_rowid FROM pages_fts_map WHERE page_id=?1)", params![page_id])?;
        db.execute("DELETE FROM pages_fts_map WHERE page_id=?1", params![page_id])?;
        db.execute("DELETE FROM page_links WHERE from_page_id=?1 OR to_page_id=?1", params![page_id])?;
    }
    Ok(())
}

/// Bound both malformed existing trees and proposed cycles; reject moves
/// whose ancestry cannot be verified rather than silently accepting them.
fn validate_parent(db: &Connection, space: &str, id: &str, parent: &str) -> Result<(), BridgeError> {
    // The space-wide record takes no children: a page under it would count as
    // a row of the Notes list's own database.
    let is_system: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM spaces WHERE system_database_id=?1)", params![parent], |row| row.get(0))?;
    if is_system {
        return Err(invalid("the Notes database is managed by Bridge"));
    }
    // A database is not a row: its own schema and its parent's would clash.
    let moved: Option<String> = db
        .query_row("SELECT kind FROM pages WHERE id=?1 AND deleted_at IS NULL", params![id], |row| row.get(0))
        .optional()?;
    if moved.as_deref() == Some("database") {
        let parent_kind: Option<String> = db
            .query_row("SELECT kind FROM pages WHERE id=?1 AND space_id=?2 AND deleted_at IS NULL", params![parent, space], |row| row.get(0))
            .optional()?;
        if parent_kind.as_deref() == Some("database") {
            return Err(invalid("a database cannot be a row of another database"));
        }
    }
    let mut next = Some(parent.to_owned());
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
        let Some(current) = next else { return Ok(()); };
        if current == id || !seen.insert(current.clone()) {
            return Err(invalid("a page cannot be its own ancestor"));
        }
        let row: Option<(Option<String>, String)> = db.query_row(
            "SELECT parent_id, kind FROM pages WHERE id=?1 AND space_id=?2 AND deleted_at IS NULL",
            params![current, space], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let (ancestor, kind) = row.ok_or_else(|| invalid("parent page not found"))?;
        if kind == "task" { return Err(invalid("a task cannot be a parent page")); }
        next = ancestor;
    }
    if next.is_none() { Ok(()) } else { Err(invalid("parent ancestry exceeds 64 pages")) }
}

fn read_structured(db: &Connection, page_id: &str, current_hash: &str) -> Result<Option<StructuredView>, BridgeError> {
    let row: Option<(String, String)> = db
        .query_row("SELECT body_hash, view FROM structured_views WHERE page_id=?1", params![page_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    Ok(row.and_then(|(hash, view)| {
        let mut view: StructuredView = serde_json::from_str(&view).ok()?;
        view.stale = hash != current_hash;
        Some(view)
    }))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The wall time a clock stamp was taken at.
fn stamp_time(stamp: &str) -> String {
    let millis = stamp.split('-').next().and_then(|millis| millis.parse::<i64>().ok()).unwrap_or(0);
    Utc.timestamp_millis_opt(millis).single().unwrap_or_else(Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn check_kind(kind: &str) -> Result<(), BridgeError> {
    if PAGE_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(invalid(format!("kind must be one of {}", PAGE_KINDS.join(", "))))
    }
}

fn check_state(state: &str) -> Result<(), BridgeError> {
    if STATES.contains(&state) {
        Ok(())
    } else {
        Err(invalid(format!("state must be one of {}", STATES.join(", "))))
    }
}

/// Lowercase slugs, deduplicated, in order.
pub(crate) fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim().trim_start_matches('#').to_lowercase();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Apply targeted replacements in order; each must match exactly once.
fn apply_edits(body: &str, edits: &[NoteTextEdit]) -> Result<String, BridgeError> {
    let mut text = body.to_owned();
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_str.is_empty() {
            return Err(invalid(format!("edit {} has an empty oldStr", index + 1)));
        }
        match text.matches(edit.old_str.as_str()).count() {
            0 => return Err(invalid(format!("edit {}: oldStr was not found; read the page and quote it exactly", index + 1))),
            1 => text = text.replacen(edit.old_str.as_str(), &edit.new_str, 1),
            n => return Err(invalid(format!("edit {}: oldStr matches {n} places; quote more of the surrounding text", index + 1))),
        }
    }
    Ok(text)
}

/// Child page and database tags the edit would drop, unless allowed.
fn check_children_kept(before: &str, after: &str, allow: bool) -> Result<(), BridgeError> {
    if allow {
        return Ok(());
    }
    let pattern = regex::Regex::new(r"<(?:page|database)\b[^>]*>").expect("valid pattern");
    let dropped: Vec<&str> = pattern.find_iter(before).map(|found| found.as_str()).filter(|tag| !after.contains(tag)).collect();
    if dropped.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "this edit would delete {} child page or database tag(s): {}. Ask the person first, then retry with allowDeletingChildren.",
            dropped.len(),
            dropped.join(", ")
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::notes::doc::ViewType;

    pub(crate) fn service() -> (tempfile::TempDir, NotesService) {
        let dir = tempfile::tempdir().unwrap();
        let service = NotesService::open(dir.path(), Arc::new(|workspace: &str| workspace == "ws-known")).unwrap();
        (dir, service)
    }

    fn edit(id: &str) -> EditPage {
        EditPage { id: id.into(), ..Default::default() }
    }

    fn page(notes: &NotesService, title: &str, body: &str) -> Note {
        notes.create(CreatePage { title: Some(title.into()), content: Some(body.into()), ..Default::default() }, PERSON).unwrap()
    }

    fn search(notes: &NotesService, query: &str) -> Vec<Note> {
        notes.list(&ListFilter { query: Some(query.into()), ..Default::default() }).unwrap()
    }

    #[test]
    fn fts5_search_ranks_title_and_stems_words() {
        let (_dir, notes) = service();
        let title = page(&notes, "Running", "ordinary text");
        let body = page(&notes, "Body hit", "running");
        assert_eq!(search(&notes, "runs").iter().map(|n| &n.id).collect::<Vec<_>>(), [&title.id, &body.id]);
        assert_eq!(search(&notes, "ru").len(), 2);
        assert!(search(&notes, "running OR missing").is_empty());
        let tagged = notes.create(CreatePage { title: Some("Tagged".into()), tags: vec!["deployment".into()], ..Default::default() }, PERSON).unwrap();
        assert_eq!(search(&notes, "depl")[0].id, tagged.id);
        let punctuation = page(&notes, "Punctuation", "100%_!");
        assert_eq!(search(&notes, "%_!")[0].id, punctuation.id);
        notes.edit(EditPage { id: body.id.clone(), text: Some("replaced".into()), ..Default::default() }, FILE_IMPORT).unwrap();
        assert_eq!(search(&notes, "runs").len(), 1);
        notes.delete(&title.id, PERSON).unwrap();
        assert!(search(&notes, "runs").is_empty());
    }

    #[test]
    fn wiki_links_and_page_tags_populate_backlinks_and_resolve_late_pages() {
        let (_dir, notes) = service();
        let source = page(&notes, "Source", "See [[Future Page|the plan]] and `[[Hidden]]`.");
        let target = page(&notes, "Future Page", "[[Future Page]]");
        let back = notes.read(&target.id).unwrap().backlinks;
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].page_id, source.id);
        assert_eq!(back[0].excerpt, "See the plan and `Hidden`.");
        let tagged = page(&notes, "Child link", &format!("<page url=\"bridge://p/{}\">Future Page</page>", target.id));
        assert_eq!(notes.read(&target.id).unwrap().backlinks.len(), 2);
        assert!(notes.read(&target.id).unwrap().backlinks.iter().any(|b| b.page_id == tagged.id && b.kind == "page"));
        let derived_source = page(&notes, "Derived source", "[[Derived title]]");
        let derived = notes.capture("# Derived title\nbody", PERSON).unwrap();
        assert_eq!(notes.read(&derived.id).unwrap().backlinks[0].page_id, derived_source.id);
        notes.delete(&source.id, PERSON).unwrap();
        assert_eq!(notes.read(&target.id).unwrap().backlinks.len(), 1);
        // A title can be assigned after the page itself exists.
        let late = notes.capture("No title yet", PERSON).unwrap();
        let late_source = page(&notes, "Late source", "[[Assigned title]]");
        notes.update(UpdateNote { id: late.id.clone(), title: Some("Assigned title".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(notes.read(&late.id).unwrap().backlinks[0].page_id, late_source.id);
    }

    #[test]
    fn renaming_a_page_updates_wiki_links_and_page_tags_in_backlinked_pages() {
        let (_dir, notes) = service();
        let target = page(&notes, "Launch Plan", "target");
        let source = page(&notes, "Source", &format!("[[Launch Plan]] [[launch plan|the plan]]\r\n<page url=\"bridge://p/{}\">Launch Plan</page>\n`[[Launch Plan]]`", target.id));
        let actor = "agent:opencode:rename";
        notes.update(UpdateNote { id: target.id.clone(), title: Some("Q4 & Launch".into()), ..Default::default() }, actor).unwrap();
        let source = notes.read(&source.id).unwrap();
        assert_eq!(source.markdown, format!("[[Q4 & Launch]] [[Q4 & Launch|the plan]]\r\n<page url=\"bridge://p/{}\">Q4 &amp; Launch</page>\n`[[Launch Plan]]`", target.id));
        assert_eq!(source.note.last_actor, actor);
        assert_eq!(notes.history(&source.note.id).unwrap().versions.len(), 2);
        let mirror = std::fs::read_to_string(source.note.path.unwrap()).unwrap();
        assert!(mirror.ends_with(&source.markdown));
        assert_eq!(notes.read(&target.id).unwrap().backlinks.len(), 2);
    }

    fn rename(notes: &NotesService, id: &str, title: &str) {
        notes.update(UpdateNote { id: id.into(), title: Some(title.into()), ..Default::default() }, PERSON).unwrap();
    }

    #[test]
    fn deleting_a_parent_keeps_a_task_moved_away_mid_delete() {
        let (_dir, notes) = service();
        let notes = Arc::new(notes);
        let parent = page(&notes, "P", "");
        let task = |title: &str| notes.create(CreatePage { title: Some(title.into()), kind: Some("task".into()), parent_id: Some(parent.id.clone()), ..Default::default() }, PERSON).unwrap();
        let (t1, t2) = (task("T1"), task("T2"));
        let q = page(&notes, "Q", "");
        let (hook, moved, t2_id, q_id) = (Arc::clone(&notes), Arc::new(std::sync::atomic::AtomicBool::new(false)), t2.id.clone(), q.id.clone());
        let t1_id = t1.id.clone();
        notes.set_listener(Arc::new(move |change: &NotePageChanged| {
            if change.page_id == t1_id && !moved.swap(true, std::sync::atomic::Ordering::SeqCst) {
                hook.update(UpdateNote { id: t2_id.clone(), parent_id: Some(q_id.clone()), ..Default::default() }, PERSON).unwrap();
            }
        }));
        notes.delete(&parent.id, PERSON).unwrap();
        assert!(notes.read(&t1.id).is_err());
        assert_eq!(notes.read(&t2.id).unwrap().note.parent_id.as_deref(), Some(q.id.as_str()));
    }

    #[test]
    fn renaming_a_duplicate_title_leaves_links_resolved_elsewhere() {
        let (_dir, notes) = service();
        let a = page(&notes, "Old", "older");
        let b = page(&notes, "Old", "newer");
        let source = page(&notes, "S", &format!("[[Old]] <page url=\"bridge://p/{}\">Old</page>", b.id));
        rename(&notes, &b.id, "New");
        assert_eq!(notes.read(&source.id).unwrap().markdown, format!("[[Old]] <page url=\"bridge://p/{}\">New</page>", b.id));
        assert_eq!(notes.read(&a.id).unwrap().backlinks[0].page_id, source.id);
    }

    #[test]
    fn overlapping_renames_leave_links_at_the_final_title() {
        let (_dir, notes) = service();
        let notes = Arc::new(notes);
        let target = page(&notes, "Old", "");
        let source = page(&notes, "S", "[[Old]]");
        let (hook, target_id) = (Arc::clone(&notes), target.id.clone());
        let second = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&second);
        notes.set_listener(Arc::new(move |change: &NotePageChanged| {
            let mut slot = slot.lock().unwrap();
            if change.page_id == target_id && slot.is_none() {
                let (notes, id) = (Arc::clone(&hook), target_id.clone());
                *slot = Some(std::thread::spawn(move || rename(&notes, &id, "Final")));
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }));
        rename(&notes, &target.id, "Mid");
        let handle = second.lock().unwrap().take().unwrap();
        handle.join().unwrap();
        assert_eq!(notes.read(&source.id).unwrap().markdown, "[[Final]]");
        assert_eq!(notes.read(&target.id).unwrap().backlinks[0].page_id, source.id);
    }

    #[test]
    fn renaming_to_a_title_wiki_links_cannot_hold_keeps_the_backlink() {
        let (_dir, notes) = service();
        let target = page(&notes, "Old", "");
        let source = page(&notes, "S", "[[Old|my alias]] [[Old]]");
        rename(&notes, &target.id, "Research | Planning");
        let tag = |text: &str| format!("<page url=\"bridge://p/{}\">{text}</page>", target.id);
        assert_eq!(notes.read(&source.id).unwrap().markdown, format!("{} {}", tag("my alias"), tag("Research | Planning")));
        assert_eq!(notes.read(&target.id).unwrap().backlinks[0].page_id, source.id);
    }

    #[test]
    fn renames_leave_quoted_and_indented_code_alone() {
        let (_dir, notes) = service();
        let target = page(&notes, "Old", "");
        let body = "> ~~~md\n> [[Old]]\n> ~~~\n\n    [[Old]]\n\n[[Old]]";
        let source = page(&notes, "S", body);
        rename(&notes, &target.id, "New");
        assert_eq!(notes.read(&source.id).unwrap().markdown, "> ~~~md\n> [[Old]]\n> ~~~\n\n    [[Old]]\n\n[[New]]");
    }

    #[test]
    fn unmatched_backticks_in_other_paragraphs_do_not_hide_links() {
        let (_dir, notes) = service();
        let target = page(&notes, "Old", "");
        let source = page(&notes, "S", "An unmatched ` here.\n\n[[Old]]\n\nAnother unmatched ` here.");
        assert_eq!(notes.read(&target.id).unwrap().backlinks[0].page_id, source.id);
        rename(&notes, &target.id, "New");
        assert!(notes.read(&source.id).unwrap().markdown.contains("[[New]]"));
    }

    #[test]
    fn a_decomposed_title_finds_its_page() {
        let (_dir, notes) = service();
        let decomposed = "e\u{301}cole";
        let page = page(&notes, decomposed, "");
        assert_eq!(search(&notes, decomposed)[0].id, page.id);
        assert_eq!(search(&notes, "école")[0].id, page.id);
    }

    #[test]
    fn search_rows_are_replaced_by_rowid_not_scanned() {
        let (_dir, notes) = service();
        let plan: String = notes.with_db(|db| Ok(db.query_row("EXPLAIN QUERY PLAN DELETE FROM pages_fts WHERE rowid=(SELECT fts_rowid FROM pages_fts_map WHERE page_id='x')", [], |r| r.get(3))?)).unwrap();
        // `0:=` is a rowid lookup; a scan of the UNINDEXED id has no constraint.
        assert!(plan.ends_with("INDEX 0:="), "{plan}");
        let a = page(&notes, "Alpha", "first");
        notes.edit(EditPage { id: a.id.clone(), text: Some("second".into()), ..Default::default() }, PERSON).unwrap();
        assert!(search(&notes, "first").is_empty());
        assert_eq!(search(&notes, "second")[0].id, a.id);
    }

    #[test]
    fn reparenting_a_page_moves_it_and_refuses_cycles() {
        let (_dir, notes) = service();
        let a = page(&notes, "A", "a");
        let b = notes.capture("b", PERSON).unwrap();
        let moved = notes.update(UpdateNote { id: b.id.clone(), parent_id: Some(a.id.clone()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(moved.parent_id.as_deref(), Some(a.id.as_str()));
        assert_eq!(moved.state, "open");
        assert!(matches!(notes.update(UpdateNote { id: a.id.clone(), parent_id: Some(b.id.clone()), ..Default::default() }, PERSON), Err(BridgeError::Invalid(_))));
        for parent in [a.id.clone(), "missing".into()] {
            assert!(notes.update(UpdateNote { id: a.id.clone(), parent_id: Some(parent), ..Default::default() }, PERSON).is_err());
        }
        let task = notes.create(CreatePage { kind: Some("task".into()), ..Default::default() }, PERSON).unwrap();
        assert!(notes.update(UpdateNote { id: b.id.clone(), parent_id: Some(task.id), ..Default::default() }, PERSON).is_err());
        let deleted = page(&notes, "Deleted", "");
        notes.delete(&deleted.id, PERSON).unwrap();
        assert!(notes.update(UpdateNote { id: b.id.clone(), parent_id: Some(deleted.id), ..Default::default() }, PERSON).is_err());
        let root = notes.update(UpdateNote { id: b.id.clone(), clear_parent: true, ..Default::default() }, PERSON).unwrap();
        assert_eq!(root.parent_id, None);
        assert!(notes.update(UpdateNote { id: b.id.clone(), clear_parent: true, parent_id: Some(a.id), ..Default::default() }, PERSON).is_err());
        let inbox = notes.capture("promote", PERSON).unwrap();
        assert_eq!(notes.update(UpdateNote { id: inbox.id, clear_parent: true, ..Default::default() }, PERSON).unwrap().state, "open");
    }

    #[test]
    fn imported_files_refresh_search_links_and_attributed_renames() {
        let (_dir, notes) = service();
        let target = page(&notes, "Target", "target body");
        let source = page(&notes, "Source", "initial");
        let path = source.path.as_deref().unwrap();
        let file = std::fs::read_to_string(path).unwrap();
        std::fs::write(path, file.replace("initial", "imported [[Target]]")).unwrap();
        assert_eq!(projector::import_changes(&notes).unwrap(), 1);
        assert_eq!(search(&notes, "import")[0].id, source.id);
        assert_eq!(notes.read(&target.id).unwrap().backlinks[0].page_id, source.id);
        let target_path = target.path.as_deref().unwrap();
        let file = std::fs::read_to_string(target_path).unwrap();
        std::fs::write(target_path, file.replace("title: \"Target\"", "title: \"Renamed\"")).unwrap();
        projector::import_changes(&notes).unwrap();
        let source = notes.read(&source.id).unwrap();
        assert_eq!(source.markdown, "imported [[Renamed]]");
        assert_eq!(source.note.last_actor, FILE_IMPORT);
    }

    #[test]
    fn parent_guard_rejects_foreign_corrupt_and_excessively_deep_trees() {
        let (_dir, notes) = service();
        let child = page(&notes, "Child", "");
        let parent = page(&notes, "Parent", "");
        notes.with_db(|db| {
            db.execute("INSERT INTO spaces(id, name, kind, created_at) VALUES('foreign', 'Other', 'team', 'now')", [])?;
            db.execute("UPDATE pages SET space_id='foreign' WHERE id=?1", [&parent.id])?;
            Ok(())
        }).unwrap();
        assert!(notes.update(UpdateNote { id: child.id.clone(), parent_id: Some(parent.id.clone()), ..Default::default() }, PERSON).is_err());
        notes.with_db(|db| {
            db.execute("UPDATE pages SET space_id=?2, parent_id=id WHERE id=?1", params![parent.id, notes.space])?;
            Ok(())
        }).unwrap();
        assert!(notes.update(UpdateNote { id: child.id.clone(), parent_id: Some(parent.id.clone()), ..Default::default() }, PERSON).is_err());
        notes.with_db(|db| { db.execute("UPDATE pages SET parent_id=NULL WHERE id=?1", [&parent.id])?; Ok(()) }).unwrap();
        let mut previous = parent.id;
        for n in 0..64 {
            previous = notes.create(CreatePage { title: Some(format!("Level {n}")), parent_id: Some(previous), ..Default::default() }, PERSON).unwrap().id;
        }
        assert!(notes.update(UpdateNote { id: child.id, parent_id: Some(previous), ..Default::default() }, PERSON).is_err());
    }

    #[test]
    fn concurrent_parent_moves_cannot_create_a_cycle() {
        let (_dir, notes) = service();
        let a = page(&notes, "A", "");
        let b = page(&notes, "B", "");
        let notes = Arc::new(notes);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [(a.id.clone(), b.id.clone()), (b.id, a.id)].into_iter().map(|(id, parent)| {
            let notes = notes.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                notes.update(UpdateNote { id, parent_id: Some(parent), ..Default::default() }, PERSON).is_ok()
            })
        }).collect();
        assert_eq!(workers.into_iter().map(|w| w.join().unwrap()).filter(|ok| *ok).count(), 1);
    }

    #[test]
    fn rebuild_index_restores_fts_and_backlinks() {
        let (_dir, notes) = service();
        let source = page(&notes, "Source", "[[Target]]");
        let target = page(&notes, "Target", "searchable");
        let before = notes.read(&target.id).unwrap().backlinks;
        notes.with_db(|db| { db.execute("DELETE FROM page_links", [])?; db.execute("DELETE FROM pages_fts", [])?; Ok(()) }).unwrap();
        notes.rebuild_index().unwrap();
        assert_eq!(search(&notes, "search")[0].id, target.id);
        let after = notes.read(&target.id).unwrap().backlinks;
        assert_eq!(after[0].page_id, source.id);
        assert_eq!(after[0].excerpt, before[0].excerpt);
        notes.rebuild_index().unwrap();
        assert_eq!(notes.read(&target.id).unwrap().backlinks, after);
    }

    #[test]
    fn capture_stores_the_body_byte_for_byte() {
        let (_dir, notes) = service();
        let typed = "  helo wrld!! idea: a notes app,\n\tkeep my typos ✨  \n";
        let note = notes.capture(typed, PERSON).unwrap();
        assert_eq!(note.state, "inbox");
        assert_eq!(note.kind, "note");
        assert_eq!(note.workspace_id, None);
        assert_eq!(note.created_by, PERSON);
        assert_eq!(notes.read(&note.id).unwrap().markdown, typed);
        let jobs: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM model_jobs", [], |row| row.get(0))?)).unwrap();
        assert_eq!(jobs, 0, "capture must not queue a model pass");
    }

    #[test]
    fn capture_refuses_an_empty_body() {
        let (_dir, notes) = service();
        assert!(notes.capture("", PERSON).is_err());
        assert!(notes.capture(" \n\t ", PERSON).is_err());
        assert!(notes.list(&ListFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn create_without_a_project_is_allowed_and_unknown_project_is_refused() {
        let (_dir, notes) = service();
        let free = notes.create(CreatePage { title: Some("No project".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(free.workspace_id, None);
        assert_eq!(free.state, "open");
        let linked = notes.create(CreatePage { title: Some("Linked".into()), workspace_id: Some("ws-known".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(linked.workspace_id.as_deref(), Some("ws-known"));
        assert!(notes.create(CreatePage { workspace_id: Some("ws-missing".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.create(CreatePage { parent_id: Some("no-such-page".into()), ..Default::default() }, PERSON).is_err());
        let cleared = notes.update(UpdateNote { id: linked.id.clone(), clear_workspace: true, ..Default::default() }, PERSON).unwrap();
        assert_eq!(cleared.workspace_id, None);
    }

    #[test]
    fn targeted_edit_requires_a_unique_match_and_is_all_or_nothing() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { content: Some("- [ ] buy domain\n- [ ] buy domain\n- [ ] write landing copy\n".into()), ..Default::default() }, PERSON).unwrap();
        let ambiguous = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "buy domain".into(), new_str: "buy bridge.dev".into() }], ..edit(&page.id) }, "agent:claude:s1");
        assert!(ambiguous.unwrap_err().to_string().contains("matches 2 places"));
        let partly_missing = notes.edit(
            EditPage {
                edits: vec![
                    NoteTextEdit { old_str: "write landing copy".into(), new_str: "write landing copy (draft)".into() },
                    NoteTextEdit { old_str: "not on the page".into(), new_str: "x".into() },
                ],
                ..edit(&page.id)
            },
            "agent:claude:s1",
        );
        assert!(partly_missing.is_err());
        assert_eq!(notes.read(&page.id).unwrap().markdown, "- [ ] buy domain\n- [ ] buy domain\n- [ ] write landing copy\n");
        let done = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "- [ ] write landing copy".into(), new_str: "- [x] write landing copy".into() }], ..edit(&page.id) }, "agent:claude:s1").unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "- [ ] buy domain\n- [ ] buy domain\n- [x] write landing copy\n");
        assert_eq!(done.note.last_actor, "agent:claude:s1");
    }

    #[test]
    fn edit_refuses_to_remove_a_child_page_tag() {
        let (_dir, notes) = service();
        let body = "Research\n\n<page url=\"bridge://p/123\">Sync libraries</page>\n";
        let page = notes.create(CreatePage { content: Some(body.into()), ..Default::default() }, PERSON).unwrap();
        let removal = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "<page url=\"bridge://p/123\">Sync libraries</page>\n".into(), new_str: String::new() }], ..edit(&page.id) }, "agent:claude:s1");
        assert!(removal.unwrap_err().to_string().contains("allowDeletingChildren"));
        let save = notes.edit(EditPage { text: Some("Research\n".into()), base_seq: Some(page.seq), ..edit(&page.id) }, PERSON);
        assert!(save.is_err());
        assert_eq!(notes.read(&page.id).unwrap().markdown, body);
        notes.edit(EditPage { text: Some("Research\n".into()), base_seq: Some(page.seq), allow_deleting_children: true, ..edit(&page.id) }, PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "Research\n");
    }

    #[test]
    fn a_stale_save_keeps_children_added_since_its_base() {
        let (_dir, notes) = service();
        let page = notes.capture("plan\n", PERSON).unwrap();
        let child = "<page url=\"bridge://p/1\">Child</page>";
        notes.edit(EditPage { append: Some(NoteAppend { content: child.into(), position: "end".into() }), ..edit(&page.id) }, "agent:claude:s1").unwrap();
        // The person's editor still holds the original text at its original base.
        let unchanged = notes.edit(EditPage { base_seq: Some(page.seq), text: Some("plan\n".into()), ..edit(&page.id) }, PERSON).unwrap();
        assert_eq!(unchanged.warnings, vec!["nothing changed"]);
        notes.edit(EditPage { base_seq: Some(page.seq), text: Some("plan v2\n".into()), ..edit(&page.id) }, PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, format!("plan v2\n{child}"));
        // A save that really removes the child is still refused.
        let current = notes.read(&page.id).unwrap();
        let removal = notes.edit(EditPage { base_seq: Some(current.seq), text: Some("plan v2\n".into()), ..edit(&page.id) }, PERSON);
        assert!(removal.unwrap_err().to_string().contains("allowDeletingChildren"));
    }

    #[test]
    fn concurrent_writes_leave_the_file_at_the_newest_version() {
        let mut stale = 0;
        for _ in 0..30 {
            let (_dir, notes) = service();
            let notes = Arc::new(notes);
            let note = notes.create(CreatePage { content: Some("original\n".into()), ..Default::default() }, PERSON).unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let writers: Vec<_> = ["first\n", "second\n"]
                .into_iter()
                .map(|line| {
                    let (notes, barrier, id) = (notes.clone(), barrier.clone(), note.id.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        notes.edit(EditPage { id, append: Some(NoteAppend { content: line.into(), position: "end".into() }), ..Default::default() }, PERSON).unwrap();
                    })
                })
                .collect();
            for writer in writers {
                writer.join().unwrap();
            }
            let page = notes.read(&note.id).unwrap();
            let file = std::fs::read_to_string(page.note.path.clone().unwrap()).unwrap();
            stale += usize::from(super::super::markdown::parse(&file).1 != page.markdown);
        }
        assert_eq!(stale, 0, "the mirror fell behind the database in {stale}/30 runs");
    }

    #[test]
    fn append_never_rewrites_existing_text() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { content: Some("first line".into()), ..Default::default() }, PERSON).unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "- added at the end".into(), position: "end".into() }), ..edit(&page.id) }, "agent:codex:s2").unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "# Heading".into(), position: "start".into() }), ..edit(&page.id) }, "agent:codex:s2").unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "# Heading\nfirst line\n- added at the end");
        assert!(notes.edit(EditPage { append: Some(NoteAppend { content: "x".into(), position: "middle".into() }), ..edit(&page.id) }, PERSON).is_err());
    }

    #[test]
    fn update_props_leaves_the_body_untouched() {
        let (_dir, notes) = service();
        let page = notes.capture("raw words, untouched", PERSON).unwrap();
        let updated = notes
            .update(UpdateNote { id: page.id.clone(), title: Some("  Titled  ".into()), kind: Some("idea".into()), tags: Some(vec!["#Launch".into(), "launch".into(), "Q4".into()]), state: Some("open".into()), ..Default::default() }, PERSON)
            .unwrap();
        assert_eq!(updated.title.as_deref(), Some("Titled"));
        assert_eq!(updated.kind, "idea");
        assert_eq!(updated.tags, vec!["launch", "q4"]);
        assert_eq!(notes.read(&page.id).unwrap().markdown, "raw words, untouched");
        assert!(notes.update(UpdateNote { id: page.id.clone(), kind: Some("essay".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.update(UpdateNote { id: page.id, state: Some("someday".into()), ..Default::default() }, PERSON).is_err());
    }

    #[test]
    fn add_session_id_is_a_set() {
        let (_dir, notes) = service();
        let page = notes.capture("start a chat from me", PERSON).unwrap();
        for session in ["s1", "s2", "s1"] {
            notes.update(UpdateNote { id: page.id.clone(), add_session_id: Some(session.into()), ..Default::default() }, PERSON).unwrap();
        }
        assert_eq!(notes.read(&page.id).unwrap().note.session_ids, vec!["s1", "s2"]);
    }

    #[test]
    fn linking_a_chat_starts_only_open_tasks_and_preserves_explicit_states() {
        let (_dir, notes) = service();
        for (state, expected) in [("inbox", "doing"), ("open", "doing"), ("doing", "doing"), ("done", "done"), ("archived", "archived")] {
            let task = notes.create(CreatePage { kind: Some("task".into()), state: Some(state.into()), ..Default::default() }, PERSON).unwrap();
            let linked = notes.update(UpdateNote { id: task.id, add_session_id: Some("chat-started".into()), ..Default::default() }, PERSON).unwrap();
            assert_eq!(linked.state, expected);
            assert_eq!(linked.session_ids, vec!["chat-started"]);
        }
        let task = notes.create(CreatePage { kind: Some("task".into()), ..Default::default() }, PERSON).unwrap();
        let linked = notes.update(UpdateNote { id: task.id, state: Some("done".into()), add_session_id: Some("chat-started".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(linked.state, "done");
        let page = notes.capture("normal note", PERSON).unwrap();
        assert_eq!(notes.update(UpdateNote { id: page.id, add_session_id: Some("chat-started".into()), ..Default::default() }, PERSON).unwrap().state, "inbox");
    }

    #[test]
    fn delete_is_a_tombstone_and_takes_its_tasks() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Launch".into()), content: Some("plan".into()), ..Default::default() }, PERSON).unwrap();
        let task = notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), ..Default::default() }, PERSON).unwrap();
        let other = notes.capture("keep me", PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().tasks.len(), 1);
        notes.delete(&page.id, PERSON).unwrap();
        assert!(notes.read(&page.id).is_err());
        assert!(notes.read(&task.id).is_err());
        let listed: Vec<String> = notes.list(&ListFilter::default()).unwrap().into_iter().map(|note| note.id).collect();
        assert_eq!(listed, vec![other.id]);
        let log_rows: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM page_updates WHERE page_id=?1", params![page.id], |row| row.get(0))?)).unwrap();
        assert_eq!(log_rows, 2, "a delete is one more logged change, not an erase");
    }

    #[test]
    fn every_write_publishes_page_changed() {
        let (_dir, notes) = service();
        let seen = Arc::new(Mutex::new(Vec::<NotePageChanged>::new()));
        let sink = seen.clone();
        notes.set_listener(Arc::new(move |change: &NotePageChanged| sink.lock().unwrap().push(change.clone())));
        let page = notes.capture("one", PERSON).unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "two".into(), position: "end".into() }), ..edit(&page.id) }, "agent:claude:s9").unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("T".into()), ..Default::default() }, PERSON).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("T".into()), ..Default::default() }, PERSON).unwrap(); // no change, no event
        let seen = seen.lock().unwrap();
        let summary: Vec<(String, i64)> = seen.iter().map(|change| (change.actor.clone(), change.seq)).collect();
        assert_eq!(summary, vec![(PERSON.into(), 1), ("agent:claude:s9".into(), 2), (PERSON.into(), 3)]);
        assert!(seen.iter().all(|change| change.page_id == page.id));
    }

    #[test]
    fn list_filters_by_text_tag_kind_state_and_parent() {
        let (_dir, notes) = service();
        let launch = notes.create(CreatePage { title: Some("Launch plan".into()), content: Some("ship 100% of it".into()), tags: vec!["launch".into()], ..Default::default() }, PERSON).unwrap();
        let task = notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(launch.id.clone()), ..Default::default() }, PERSON).unwrap();
        let idea = notes.capture("an idea about sync_engines", PERSON).unwrap();
        let archived = notes.capture("old stuff", PERSON).unwrap();
        notes.update(UpdateNote { id: archived.id.clone(), state: Some("archived".into()), ..Default::default() }, PERSON).unwrap();
        let ids = |filter: ListFilter| -> Vec<String> { notes.list(&filter).unwrap().into_iter().map(|note| note.id).collect() };
        assert_eq!(ids(ListFilter { query: Some("launch PLAN".into()), ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { query: Some("100%".into()), ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { query: Some("sync_e".into()), ..Default::default() }), vec![idea.id.clone()]);
        // Punctuation separates FTS tokens; it is never passed through as
        // LIKE wildcards or FTS operators.
        assert_eq!(ids(ListFilter { query: Some("sync%e".into()), ..Default::default() }), vec![idea.id.clone()]);
        assert_eq!(ids(ListFilter { tags: vec!["#Launch".into()], ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { kind: Some("task".into()), ..Default::default() }), vec![task.id.clone()]);
        assert_eq!(ids(ListFilter { parent_id: Some(launch.id.clone()), ..Default::default() }), vec![task.id.clone()]);
        assert_eq!(ids(ListFilter { state: Some("inbox".into()), ..Default::default() }), vec![idea.id.clone()]);
        assert!(!ids(ListFilter::default()).contains(&archived.id));
        assert!(ids(ListFilter { include_archived: true, ..Default::default() }).contains(&archived.id));
        assert_eq!(ids(ListFilter { state: Some("archived".into()), ..Default::default() }), vec![archived.id]);
    }

    #[test]
    fn rebuild_index_from_updates_restores_everything() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Launch".into()), content: Some("plan it".into()), tags: vec!["launch".into()], workspace_id: Some("ws-known".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), source_excerpt: Some("plan it".into()), ..Default::default() }, "model:tasks:sonnet").unwrap();
        notes.update(UpdateNote { id: page.id.clone(), add_session_id: Some("s1".into()), state: Some("doing".into()), ..Default::default() }, PERSON).unwrap();
        let gone = notes.capture("delete me", PERSON).unwrap();
        notes.delete(&gone.id, PERSON).unwrap();
        let before = notes.list(&ListFilter { include_archived: true, ..Default::default() }).unwrap();
        let page_before = notes.read(&page.id).unwrap();

        notes.with_db(|db| Ok(db.execute("DELETE FROM pages", [])?)).unwrap();
        assert!(notes.list(&ListFilter::default()).unwrap().is_empty());
        assert_eq!(notes.rebuild_index().unwrap(), 4, "three pages and the space's system database");

        let mut after = notes.list(&ListFilter { include_archived: true, ..Default::default() }).unwrap();
        let mut expected = before.clone();
        let key = |note: &Note| note.id.clone();
        after.sort_by_key(key);
        expected.sort_by_key(key);
        let strip = |notes: Vec<Note>| notes.into_iter().map(|note| Note { updated_at: String::new(), ..note }).collect::<Vec<_>>();
        assert_eq!(strip(after), strip(expected));
        let page_after = notes.read(&page.id).unwrap();
        assert_eq!(page_after.markdown, page_before.markdown);
        assert_eq!(page_after.tasks.len(), 1);
        assert_eq!(page_after.note.session_ids, vec!["s1"]);
        assert!(notes.read(&gone.id).is_err());
    }

    #[test]
    fn history_lists_body_changes_with_authors_and_counts() {
        let (_dir, notes) = service();
        let page = notes.capture("helo wrld", PERSON).unwrap();
        let agent = agent_actor("claude", "s1");
        notes.edit(EditPage { append: Some(NoteAppend { content: "from the agent".into(), position: "end".into() }), ..edit(&page.id) }, &agent).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("Greeting".into()), ..Default::default() }, PERSON).unwrap();
        let current = notes.read(&page.id).unwrap();
        notes.edit(EditPage { text: Some("hello world\nfrom the agent".into()), base_seq: Some(current.seq), ..edit(&page.id) }, FILE_IMPORT).unwrap();

        let history = notes.history(&page.id).unwrap().versions;
        let actors: Vec<&str> = history.iter().map(|version| version.actor.as_str()).collect();
        assert_eq!(actors, vec![FILE_IMPORT, PERSON, agent.as_str(), PERSON], "newest first");
        let seqs: Vec<i64> = history.iter().map(|version| version.seq).collect();
        assert!(seqs.windows(2).all(|pair| pair[0] > pair[1]));
        let [import, title, append, capture] = &history[..] else { panic!("four versions") };
        assert!(capture.body_changed && capture.added == 9 && capture.removed == 0, "{capture:?}");
        assert_eq!((append.body_changed, append.added, append.removed), (true, 15, 0));
        assert!(!title.body_changed, "a title change touches no words");
        assert_eq!(title.title.as_deref(), Some("Greeting"));
        assert_eq!((import.added, import.removed), (2, 0), "helo wrld -> hello world");
        assert!(chrono::DateTime::parse_from_rfc3339(&capture.at).is_ok(), "{}", capture.at);
    }

    #[test]
    fn history_window_is_the_newest_two_hundred() {
        let (_dir, notes) = service();
        let page = notes.capture("0", PERSON).unwrap();
        for index in 1..=210 {
            notes.edit(EditPage { append: Some(NoteAppend { content: format!("{index}"), position: "end".into() }), ..edit(&page.id) }, PERSON).unwrap();
        }
        let history = notes.history(&page.id).unwrap().versions;
        assert_eq!(history.len(), 200);
        assert_eq!(history[0].seq, notes.read(&page.id).unwrap().seq);
        assert!(history.iter().all(|version| version.body_changed && version.added > 0), "counts hold across the window and snapshots");
    }

    #[test]
    fn read_version_rebuilds_the_page_at_that_seq() {
        let (_dir, notes) = service();
        let page = notes.capture("first words", PERSON).unwrap();
        let first = notes.read(&page.id).unwrap().seq;
        notes.edit(EditPage { append: Some(NoteAppend { content: "agent line".into(), position: "end".into() }), ..edit(&page.id) }, &agent_actor("codex", "s2")).unwrap();
        let version = notes.read_version(&page.id, first).unwrap();
        assert_eq!((version.seq, version.actor.as_str(), version.markdown.as_str()), (first, PERSON, "first words"));
        assert!(notes.read_version(&page.id, 999).is_err());
        assert!(notes.read_version(&page.id, 0).is_err());
    }

    #[test]
    fn restore_writes_the_old_body_as_a_new_person_version() {
        let (_dir, notes) = service();
        let page = notes.capture("my plan", PERSON).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("Plan".into()), state: Some("open".into()), ..Default::default() }, PERSON).unwrap();
        let mine = notes.read(&page.id).unwrap().seq;
        let agent = agent_actor("claude", "s1");
        notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "my plan".into(), new_str: "the agent's plan".into() }], ..edit(&page.id) }, &agent).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), state: Some("doing".into()), ..Default::default() }, PERSON).unwrap();

        let restored = notes.restore_version(&page.id, mine, PERSON).unwrap();
        assert!(restored.warnings.is_empty());
        let now = notes.read(&page.id).unwrap();
        assert_eq!(now.markdown, "my plan");
        assert_eq!((now.note.title.as_deref(), now.note.state.as_str()), (Some("Plan"), "doing"), "only the words come back");
        assert_eq!(now.note.last_actor, PERSON);
        let history = notes.history(&page.id).unwrap().versions;
        assert_eq!(history[0].actor, PERSON);
        assert!(history.iter().any(|version| version.actor == agent), "the agent's version stays in history");

        // Restoring the agent's version undoes the restore.
        let agents = history.iter().find(|version| version.actor == agent).unwrap().seq;
        notes.restore_version(&page.id, agents, PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "the agent's plan");
        assert_eq!(notes.restore_version(&page.id, agents, PERSON).unwrap().warnings, vec!["nothing changed"]);
        assert!(notes.restore_version(&page.id, 999, PERSON).is_err());
        notes.delete(&page.id, PERSON).unwrap();
        assert!(notes.restore_version(&page.id, mine, PERSON).is_err());
    }

    /// A database with an empty record, a number property and a status
    /// property with one option.
    fn reading_list(notes: &NotesService) -> (String, String, String) {
        let id = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id;
        let updated = notes
            .update_schema(
                &id,
                &[
                    SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty {
                        name: "Status".into(),
                        kind: "status".into(),
                        options: vec![NewOption { name: "Reading".into(), ..Default::default() }, NewOption { name: "Done".into(), group: Some("done".into()), ..Default::default() }],
                        ..Default::default()
                    }),
                ],
                PERSON,
            )
            .unwrap();
        (id, updated.added[0].0.clone(), updated.added[1].0.clone())
    }

    fn values_of(notes: &NotesService, row: &str) -> std::collections::BTreeMap<String, Value> {
        notes.with_db(|db| Ok(doc::load_page(db, row, None)?.0.values())).unwrap()
    }

    #[test]
    fn create_database_seeds_status_and_a_table_view() {
        let (_dir, notes) = service();
        let note = notes.create_database(CreateDatabase { title: Some("Reading list".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(note.kind, "database");
        let database = notes.database(&note.id).unwrap();
        let (_, status) = database.ordered().into_iter().next().unwrap();
        assert_eq!((status.name.as_str(), status.kind), ("Status", PropertyType::Status));
        assert_eq!(status.options.iter().map(|option| (option.name.as_str(), option.group.as_deref())).collect::<Vec<_>>(), vec![("To do", Some("todo")), ("In progress", Some("inProgress")), ("Done", Some("done"))]);
        assert_eq!(database.views.values().map(|view| (view.name.as_str(), view.kind)).collect::<Vec<_>>(), vec![("Table", ViewType::Table)]);

        // Given properties and views replace the defaults.
        let board = ViewDef { name: "Board".into(), kind: ViewType::Board, order: 0, filter: None, sorts: vec![], group_by: Some(doc::GroupByDef { property: "state".into(), date_bucket: None }), date_property: None, properties: vec![] };
        let note = notes
            .create_database(CreateDatabase { title: Some("Books".into()), properties: vec![NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() }], views: vec![board], ..Default::default() }, PERSON)
            .unwrap();
        let database = notes.database(&note.id).unwrap();
        assert_eq!(database.schema.values().map(|def| def.name.as_str()).collect::<Vec<_>>(), vec!["Rating"]);
        assert_eq!(database.views.values().map(|view| view.name.as_str()).collect::<Vec<_>>(), vec!["Board"]);

        // A refused property leaves no half-made database behind.
        let before = notes.list(&ListFilter::default()).unwrap().len();
        assert!(notes.create_database(CreateDatabase { title: Some("Bad".into()), properties: vec![NewProperty { name: "title".into(), kind: "text".into(), ..Default::default() }], ..Default::default() }, PERSON).is_err());
        assert_eq!(notes.list(&ListFilter::default()).unwrap().len(), before);
    }

    #[test]
    fn set_options_diffs_into_add_rename_delete_and_order() {
        let (_dir, notes) = service();
        let (id, _rating, status) = reading_list(&notes);
        let options = notes.database(&id).unwrap().schema[&status].options.clone();
        let (reading, done) = (options[0].clone(), options[1].clone());
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(status.clone(), Some(serde_json::json!(reading.id)))], PERSON).unwrap();
        let done_row = notes.create(CreatePage { title: Some("SICP".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&done_row.id, &[(status.clone(), Some(serde_json::json!(done.id)))], PERSON).unwrap();

        // Done first and renamed, Reading deleted, a new Paused option in between.
        notes
            .update_schema(
                &id,
                &[SchemaOp::SetOptions {
                    prop: status.clone(),
                    options: vec![
                        OptionInput { id: Some(done.id.clone()), name: "Finished".into(), ..Default::default() },
                        OptionInput { name: "Paused".into(), group: Some("inProgress".into()), ..Default::default() },
                    ],
                    base: None,
                }],
                PERSON,
            )
            .unwrap();
        let def = notes.database(&id).unwrap().schema[&status].clone();
        assert_eq!(def.options.iter().map(|option| option.name.as_str()).collect::<Vec<_>>(), vec!["Finished", "Paused"]);
        assert_eq!(def.options[0].id, done.id, "a kept option keeps its id, so its values stay");
        assert_eq!(def.options[0].group.as_deref(), Some("done"), "a group left out stays");
        assert_eq!(def.options[1].group.as_deref(), Some("inProgress"));
        assert!(!values_of(&notes, &row.id).contains_key(&status), "a deleted option's values are cleared");
        assert_eq!(values_of(&notes, &done_row.id)[&status], serde_json::json!(done.id));

        // Two option edits on one property fold; a duplicate name or an unknown id is refused.
        assert!(notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: vec![OptionInput { name: "A".into(), ..Default::default() }, OptionInput { name: "a".into(), ..Default::default() }], base: None }], PERSON).is_err());
        assert!(notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: vec![OptionInput { id: Some("ghost".into()), name: "A".into(), ..Default::default() }], base: None }], PERSON).is_err());
        assert_eq!(notes.database(&id).unwrap().schema[&status].options.len(), 2, "a refused batch changes nothing");
    }

    #[test]
    fn set_options_with_a_stale_base_is_refused_and_writes_nothing() {
        let (_dir, notes) = service();
        let (id, _rating, status) = reading_list(&notes);
        let read: Vec<String> = notes.database(&id).unwrap().schema[&status].options.iter().map(|option| option.id.clone()).collect();
        // Someone adds an option after this caller read the list.
        notes.update_schema(&id, &[SchemaOp::AddOption { prop: status.clone(), name: "Paused".into(), color: None, group: None }], PERSON).unwrap();
        let stored = notes.database(&id).unwrap().schema[&status].options.clone();
        let mine: Vec<OptionInput> = stored.iter().filter(|option| read.contains(&option.id)).map(|option| OptionInput { id: Some(option.id.clone()), name: option.name.clone(), ..Default::default() }).chain([OptionInput { name: "Dropped".into(), ..Default::default() }]).collect();
        let refused = notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: mine.clone(), base: Some(read.clone()) }], PERSON).unwrap_err();
        assert!(refused.to_string().contains("changed since they were read"), "{refused}");
        assert_eq!(notes.database(&id).unwrap().schema[&status].options, stored, "nothing written");
        // Against the ids as stored now, the same list goes through.
        let now: Vec<String> = stored.iter().map(|option| option.id.clone()).collect();
        let mut with_paused = mine;
        with_paused.insert(with_paused.len() - 1, OptionInput { id: Some(stored.last().unwrap().id.clone()), name: "Paused".into(), ..Default::default() });
        notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: with_paused, base: Some(now) }], PERSON).unwrap();
        let names: Vec<String> = notes.database(&id).unwrap().schema[&status].options.iter().map(|option| option.name.clone()).collect();
        assert!(names.contains(&"Paused".to_owned()) && names.contains(&"Dropped".to_owned()), "{names:?}");
    }

    /// A row of the reading list with a notice on each of its two properties.
    fn row_with_notices(notes: &NotesService) -> (String, String, String, String) {
        let (id, rating, status) = reading_list(notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.with_db(|db| {
            notices::record(db, notes.space(), &row.id, &rating, "three", "number", 1)?;
            notices::record(db, notes.space(), &row.id, &status, "someday", "status", 1)
        }).unwrap();
        (id, row.id, rating, status)
    }

    #[test]
    fn setting_the_cell_clears_its_notice() {
        let (_dir, notes) = service();
        let (_db, row, rating, status) = row_with_notices(&notes);
        assert_eq!(notes.read(&row).unwrap().notices.len(), 2);
        notes.set_values(&row, &[(rating.clone(), Some(serde_json::json!(3)))], PERSON).unwrap();
        let left: Vec<String> = notes.read(&row).unwrap().notices.into_iter().map(|notice| notice.property_id).collect();
        assert_eq!(left, vec![status.clone()]);
        // Through update with values too.
        let reading = notes.database(&_db).unwrap().schema[&status].options[0].id.clone();
        notes.update(UpdateNote { id: row.clone(), values: vec![(status, Some(serde_json::json!(reading)))], ..Default::default() }, PERSON).unwrap();
        assert!(notes.read(&row).unwrap().notices.is_empty());
    }

    #[test]
    fn rebuild_index_clears_notices() {
        let (_dir, notes) = service();
        let (_db, row, _, _) = row_with_notices(&notes);
        notes.rebuild_index().unwrap();
        assert!(notes.read(&row).unwrap().notices.is_empty());
        assert_eq!(notes.with_db(|db| Ok(db.query_row("SELECT count(*) FROM page_notices", [], |r| r.get::<_, i64>(0))?)).unwrap(), 0);
    }

    #[test]
    fn deleting_a_page_deletes_its_notices() {
        let (_dir, notes) = service();
        let (_db, row, _, _) = row_with_notices(&notes);
        notes.delete(&row, PERSON).unwrap();
        assert_eq!(notes.with_db(|db| Ok(db.query_row("SELECT count(*) FROM page_notices WHERE page_id=?1", [&row], |r| r.get::<_, i64>(0))?)).unwrap(), 0);
    }

    #[test]
    fn only_a_row_reads_notices() {
        let (_dir, notes) = service();
        let (_db, row, rating, _) = row_with_notices(&notes);
        let loose = notes.create(CreatePage { title: Some("Loose".into()), ..Default::default() }, PERSON).unwrap();
        notes.with_db(|db| notices::record(db, notes.space(), &loose.id, &rating, "x", "number", 1)).unwrap();
        assert!(notes.read(&loose.id).unwrap().notices.is_empty(), "a page that is not a row reads none");
        let read = notes.read(&row).unwrap().notices;
        assert_eq!(read.len(), 2);
        assert_eq!((read[0].kind.as_str(), read.iter().any(|notice| notice.raw == "three" && notice.expected == "number")), ("unreadableValue", true));
    }

    #[test]
    fn set_options_finishes_an_interrupted_cleanup_on_retry() {
        let (_dir, notes) = service();
        let (id, _rating, status) = reading_list(&notes);
        let options = notes.database(&id).unwrap().schema[&status].options.clone();
        let (reading, done) = (options[0].clone(), options[1].clone());
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        let keep_done = || vec![OptionInput { id: Some(done.id.clone()), name: done.name.clone(), ..Default::default() }];
        notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: keep_done(), base: None }], PERSON).unwrap();
        // The state an interrupted cleanup leaves: the schema no longer has
        // Reading, but a row still names it.
        notes.write(&row.id, PERSON, |live| live.edit_records(&RecordsPatch::default().put_value(&status, Some(&serde_json::json!(reading.id))))).unwrap();
        assert_eq!(values_of(&notes, &row.id)[&status], serde_json::json!(reading.id));
        // Retrying the same request finishes it, though the schema is unchanged.
        notes.update_schema(&id, &[SchemaOp::SetOptions { prop: status.clone(), options: keep_done(), base: None }], PERSON).unwrap();
        assert!(!values_of(&notes, &row.id).contains_key(&status), "the stale option is cleared on retry");
    }

    #[test]
    fn saved_views_stay_queryable_after_schema_edits() {
        use crate::notes::doc::{GroupByDef, SortDef, ViewPropertyDef};
        use crate::notes::query::QuerySpec;
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let genre = notes
            .update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Genre".into(), kind: "select".into(), options: vec![NewOption { name: "Fiction".into(), ..Default::default() }, NewOption { name: "Tech".into(), ..Default::default() }], ..Default::default() })], PERSON)
            .unwrap()
            .added[0]
            .0
            .clone();
        let options = notes.database(&id).unwrap().schema[&genre].options.clone();
        let (fiction, tech) = (options[0].id.clone(), options[1].id.clone());
        let view = ViewDef {
            name: "Picked".into(),
            kind: ViewType::Board,
            order: 1,
            filter: Some(serde_json::json!({"and": [
                {"property": genre, "op": "is", "value": fiction},
                {"property": genre, "op": "isAnyOf", "value": [fiction, tech]},
                {"property": rating, "op": "gte", "value": 3},
            ]})),
            sorts: vec![SortDef { property: rating.clone(), direction: "desc".into() }, SortDef { property: "title".into(), direction: "asc".into() }],
            group_by: Some(GroupByDef { property: genre.clone(), date_bucket: None }),
            date_property: None,
            properties: vec![ViewPropertyDef { id: rating.clone(), visible: true, width: None }, ViewPropertyDef { id: genre.clone(), visible: true, width: None }],
        };
        let view_id = notes.put_view(&id, None, view, PERSON).unwrap();
        let query = |notes: &NotesService| notes.query_database(&id, &QuerySpec { view_id: Some(view_id.clone()), ..Default::default() });
        let saved = |notes: &NotesService| notes.database(&id).unwrap().views[&view_id].clone();

        // Removing an option drops the condition that named it alone and keeps
        // the survivor of a list.
        notes.update_schema(&id, &[SchemaOp::SetOptions { prop: genre.clone(), options: vec![OptionInput { id: Some(tech.clone()), name: "Tech".into(), ..Default::default() }], base: None }], PERSON).unwrap();
        assert!(query(&notes).is_ok(), "{:?}", query(&notes).err());
        assert_eq!(saved(&notes).filter, Some(serde_json::json!({"and": [{"property": genre, "op": "isAnyOf", "value": [tech]}, {"property": rating, "op": "gte", "value": 3}]})));

        // Deleting a property drops its condition, sort and column.
        notes.update_schema(&id, &[SchemaOp::Delete { prop: rating.clone() }], PERSON).unwrap();
        assert!(query(&notes).is_ok(), "{:?}", query(&notes).err());
        let view = saved(&notes);
        assert_eq!(view.filter, Some(serde_json::json!({"and": [{"property": genre, "op": "isAnyOf", "value": [tech]}]})));
        assert_eq!(view.sorts.iter().map(|sort| sort.property.as_str()).collect::<Vec<_>>(), vec!["title"]);
        assert_eq!(view.properties.iter().map(|property| property.id.as_str()).collect::<Vec<_>>(), vec![genre.as_str()]);

        // Retyping to text: a list op and a grouping the type cannot take go.
        notes.update_schema(&id, &[SchemaOp::Retype { prop: genre.clone(), to: "text".into() }], PERSON).unwrap();
        assert!(query(&notes).is_ok(), "{:?}", query(&notes).err());
        let view = saved(&notes);
        assert_eq!((view.filter, view.group_by), (None, None));
    }

    #[test]
    fn reorder_must_name_every_property() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        notes.update_schema(&id, &[SchemaOp::Reorder { props: vec![status.clone(), rating.clone()] }], PERSON).unwrap();
        let ordered: Vec<String> = notes.database(&id).unwrap().ordered().into_iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ordered, vec![status.clone(), rating.clone()]);
        for bad in [vec![status.clone()], vec![status.clone(), status.clone()], vec![status.clone(), "ghost".into()]] {
            let error = notes.update_schema(&id, &[SchemaOp::Reorder { props: bad }], PERSON).unwrap_err().to_string();
            assert!(error.contains("every property once"), "{error}");
        }
    }

    #[test]
    fn the_last_view_cannot_be_deleted_and_a_saved_view_must_compile() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let only = notes.database(&id).unwrap().views.into_keys().collect::<Vec<_>>();
        assert_eq!(only.len(), 1);
        let error = notes.delete_view(&id, &only[0], PERSON).unwrap_err().to_string();
        assert!(error.contains("at least one view"), "{error}");
        let view = |filter: serde_json::Value| ViewDef { name: "Mine".into(), kind: ViewType::List, order: 1, filter: Some(filter), sorts: vec![], group_by: None, date_property: None, properties: vec![] };
        let error = notes.put_view(&id, None, view(serde_json::json!({"property": rating, "op": "contains", "value": "x"})), PERSON).unwrap_err().to_string();
        assert!(error.contains("eq, ne"), "{error}");
        let mine = notes.put_view(&id, None, view(serde_json::json!({"property": rating, "op": "gte", "value": 3})), PERSON).unwrap();
        notes.delete_view(&id, &only[0], PERSON).unwrap();
        assert_eq!(notes.database(&id).unwrap().views.into_keys().collect::<Vec<_>>(), vec![mine]);
        let calendar = ViewDef { date_property: Some(rating), kind: ViewType::Calendar, ..view(serde_json::json!({"and": []})) };
        assert!(notes.put_view(&id, None, calendar, PERSON).unwrap_err().to_string().contains("not a date"));
    }

    #[test]
    fn read_carries_the_database_or_the_row_values() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let page = notes.read(&id).unwrap();
        let database = page.database.expect("a database page carries its records");
        assert_eq!(database.properties.iter().map(|property| property.name.as_str()).collect::<Vec<_>>(), vec!["Rating", "Status"]);
        assert_eq!(database.properties[0].id, rating);
        assert_eq!(database.views.len(), 1);
        assert_eq!((page.database_id, page.values), (None, None));
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        // A deleted property's cell stays in the doc but not on the page.
        notes.update_schema(&id, &[SchemaOp::Delete { prop: status.clone() }], PERSON).unwrap();
        let page = notes.read(&row.id).unwrap();
        assert_eq!(page.database_id.as_deref(), Some(id.as_str()));
        assert_eq!(page.values, Some([(rating, serde_json::json!(5))].into_iter().collect()));
        assert!(page.database.is_none());
        let plain = notes.create(CreatePage { title: Some("Plain".into()), ..Default::default() }, PERSON).unwrap();
        let page = notes.read(&plain.id).unwrap();
        assert_eq!((page.database, page.database_id, page.values), (None, None, None));
    }

    #[test]
    fn a_refused_cell_in_an_update_writes_nothing() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        let seq = notes.read(&row.id).unwrap().seq;
        let error = notes
            .update(UpdateNote { id: row.id.clone(), title: Some("Renamed".into()), state: Some("done".into()), values: vec![(rating.clone(), Some(serde_json::json!(4))), (status, Some(serde_json::json!("nope")))], ..Default::default() }, PERSON)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Reading"), "{error}");
        let page = notes.read(&row.id).unwrap();
        assert_eq!((page.seq, page.note.title.as_deref(), page.note.state.as_str()), (seq, Some("DDIA"), "open"), "neither the properties nor the cells changed");
        // Moving a page into a database and setting its cells is one request:
        // the cells are checked against the database it moves into.
        let loose = notes.create(CreatePage { title: Some("SICP".into()), ..Default::default() }, PERSON).unwrap();
        notes.update(UpdateNote { id: loose.id.clone(), parent_id: Some(id.clone()), values: vec![(rating.clone(), Some(serde_json::json!(3)))], ..Default::default() }, PERSON).unwrap();
        assert_eq!(values_of(&notes, &loose.id)[&rating], serde_json::json!(3));
        assert!(notes.update(UpdateNote { id: loose.id, clear_parent: true, values: vec![(rating, Some(serde_json::json!(1)))], ..Default::default() }, PERSON).is_err(), "a page leaving its database takes no cells");
    }

    #[test]
    fn a_move_and_its_cells_are_one_write() {
        let (_dir, notes) = service();
        let (list, _rating, _status) = reading_list(&notes);
        // A database whose relation points at itself.
        let related = notes.update_schema(&list, &[SchemaOp::Add(NewProperty { name: "Related".into(), kind: "relation".into(), relation: Some(list.clone()), ..Default::default() })], PERSON).unwrap().added[0].0.clone();
        let other = notes.create(CreatePage { title: Some("Other list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id;
        let back = notes.update_schema(&other, &[SchemaOp::Add(NewProperty { name: "Back".into(), kind: "relation".into(), relation: Some(list.clone()), ..Default::default() })], PERSON).unwrap().added[0].0.clone();

        // Moving a page in and linking it to itself: it is a row once the move lands.
        let page = notes.create(CreatePage { title: Some("Self".into()), ..Default::default() }, PERSON).unwrap();
        let seq = notes.read(&page.id).unwrap().seq;
        notes.update(UpdateNote { id: page.id.clone(), parent_id: Some(list.clone()), values: vec![(related.clone(), Some(serde_json::json!([page.id])))], ..Default::default() }, PERSON).unwrap();
        let moved = notes.read(&page.id).unwrap();
        assert_eq!(moved.seq, seq + 1, "one write");
        assert_eq!(moved.note.parent_id.as_deref(), Some(list.as_str()));
        assert_eq!(values_of(&notes, &page.id)[&related], serde_json::json!([page.id]));

        // Moving it out while pointing at itself in the old database: it is no
        // longer a row there, so the cell is refused and the move does not land.
        let error = notes.update(UpdateNote { id: page.id.clone(), parent_id: Some(other.clone()), values: vec![(back, Some(serde_json::json!([page.id])))], ..Default::default() }, PERSON).unwrap_err().to_string();
        assert!(error.contains("not a row of that database"), "{error}");
        let after = notes.read(&page.id).unwrap();
        assert_eq!((after.seq, after.note.parent_id.as_deref()), (moved.seq, Some(list.as_str())), "neither the move nor the cell landed");
    }

    #[test]
    fn initial_views_name_the_properties_being_created() {
        use crate::notes::doc::{GroupByDef, SortDef, ViewPropertyDef};
        let (_dir, notes) = service();
        let blank = |name: &str, kind: ViewType| ViewDef { name: name.into(), kind, order: 0, filter: None, sorts: vec![], group_by: None, date_property: None, properties: vec![] };
        let calendar = ViewDef { date_property: Some("Due".into()), ..blank("Calendar", ViewType::Calendar) };
        let board = ViewDef {
            filter: Some(serde_json::json!({"property": "genre", "op": "is", "value": "Fiction"})),
            sorts: vec![SortDef { property: "Due".into(), direction: "asc".into() }],
            group_by: Some(GroupByDef { property: "Genre".into(), date_bucket: None }),
            properties: vec![ViewPropertyDef { id: "Due".into(), visible: true, width: None }],
            ..blank("Board", ViewType::Board)
        };
        let note = notes
            .create_database(
                CreateDatabase {
                    title: Some("Books".into()),
                    properties: vec![
                        NewProperty { name: "Due".into(), kind: "date".into(), ..Default::default() },
                        NewProperty { name: "Genre".into(), kind: "select".into(), options: vec![NewOption { name: "Fiction".into(), ..Default::default() }], ..Default::default() },
                    ],
                    views: vec![calendar, board],
                    ..Default::default()
                },
                PERSON,
            )
            .unwrap();
        let database = notes.database(&note.id).unwrap();
        let id_of = |name: &str| database.ordered().into_iter().find(|(_, def)| def.name == name).unwrap().0.clone();
        let (due, genre) = (id_of("Due"), id_of("Genre"));
        let fiction = database.schema[&genre].options[0].id.clone();
        let view = |name: &str| database.views.values().find(|view| view.name == name).unwrap().clone();
        assert_eq!(view("Calendar").date_property, Some(due.clone()));
        let board = view("Board");
        assert_eq!(board.filter, Some(serde_json::json!({"property": genre, "op": "is", "value": fiction})), "names resolve to the generated ids");
        assert_eq!((board.sorts[0].property.clone(), board.group_by.unwrap().property, board.properties[0].id.clone()), (due.clone(), genre, due));
        assert!(notes.query_database(&note.id, &crate::notes::query::QuerySpec { view_id: Some(database.views.iter().find(|(_, view)| view.name == "Board").unwrap().0.clone()), ..Default::default() }).is_ok());
    }

    #[test]
    fn a_row_and_its_first_cells_are_one_write() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let pages = |notes: &NotesService| notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?)).unwrap();
        let before = pages(&notes);
        let refused = notes.create(CreatePage { title: Some("Book".into()), parent_id: Some(id.clone()), values: vec![(rating.clone(), Some(serde_json::json!(4))), (status, Some(serde_json::json!("nope")))], ..Default::default() }, PERSON);
        assert!(refused.is_err());
        assert_eq!(pages(&notes), before, "a refused cell makes no row");
        let row = notes.create(CreatePage { title: Some("Book".into()), parent_id: Some(id.clone()), values: vec![(rating.clone(), Some(serde_json::json!(4)))], ..Default::default() }, PERSON).unwrap();
        assert_eq!(notes.history(&row.id).unwrap().versions.len(), 1, "made with its cells in one write");
        assert_eq!(values_of(&notes, &row.id)[&rating], serde_json::json!(4));
        let loose = notes.create(CreatePage { title: Some("Loose".into()), values: vec![(rating, Some(serde_json::json!(1)))], ..Default::default() }, PERSON);
        assert!(loose.unwrap_err().to_string().contains("only a row"));
    }

    #[test]
    fn create_database_is_one_write_that_refuses_whole() {
        let (_dir, notes) = service();
        let before = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?)).unwrap();
        let bad_view = ViewDef { name: "Bad".into(), kind: ViewType::List, order: 0, filter: Some(serde_json::json!({"property": "title", "op": "gt", "value": 1})), sorts: vec![], group_by: None, date_property: None, properties: vec![] };
        assert!(notes.create_database(CreateDatabase { title: Some("A".into()), views: vec![bad_view], ..Default::default() }, PERSON).is_err());
        assert!(notes.create_database(CreateDatabase { title: Some("B".into()), properties: vec![NewProperty { name: "A".into(), kind: "text".into(), ..Default::default() }, NewProperty { name: "a".into(), kind: "text".into(), ..Default::default() }], ..Default::default() }, PERSON).is_err());
        let after = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?)).unwrap();
        assert_eq!(after, before, "no page, not even a tombstone");
        // Fifty views fit: none is a seeded extra.
        let fifty: Vec<ViewDef> = (0..MAX_VIEWS).map(|index| ViewDef { name: format!("View {index}"), kind: ViewType::Table, order: index as i64, filter: None, sorts: vec![], group_by: None, date_property: None, properties: vec![] }).collect();
        let note = notes.create_database(CreateDatabase { title: Some("Many".into()), views: fifty, ..Default::default() }, PERSON).unwrap();
        assert_eq!(notes.database(&note.id).unwrap().views.len(), MAX_VIEWS);
        assert_eq!(notes.history(&note.id).unwrap().versions.len(), 1, "made in one write");
    }

    #[test]
    fn two_deletes_at_once_keep_a_view() {
        let (_dir, notes) = service();
        let (id, _rating, _status) = reading_list(&notes);
        let view = ViewDef { name: "Second".into(), kind: ViewType::List, order: 1, filter: None, sorts: vec![], group_by: None, date_property: None, properties: vec![] };
        notes.put_view(&id, None, view, PERSON).unwrap();
        let notes = Arc::new(notes);
        for _ in 0..10 {
            let views: Vec<String> = notes.database(&id).unwrap().views.into_keys().collect();
            if views.len() < 2 {
                let view = ViewDef { name: "Again".into(), kind: ViewType::List, order: 2, filter: None, sorts: vec![], group_by: None, date_property: None, properties: vec![] };
                notes.put_view(&id, None, view, PERSON).unwrap();
                continue;
            }
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let handles: Vec<_> = views
                .into_iter()
                .take(2)
                .map(|view| {
                    let (notes, id, barrier) = (notes.clone(), id.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        notes.delete_view(&id, &view, PERSON).is_ok()
                    })
                })
                .collect();
            let deleted = handles.into_iter().map(|handle| handle.join().unwrap()).filter(|ok| *ok).count();
            assert_eq!(deleted, 1, "exactly one of two racing deletes wins");
            assert_eq!(notes.database(&id).unwrap().views.len(), 1);
        }
    }

    #[test]
    fn update_with_values_sets_and_clears() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.update(UpdateNote { id: row.id.clone(), values: vec![(rating.clone(), Some(serde_json::json!(4)))], ..Default::default() }, PERSON).unwrap();
        assert_eq!(values_of(&notes, &row.id)[&rating], serde_json::json!(4));
        // Properties and cells in one call, and a clear.
        let note = notes.update(UpdateNote { id: row.id.clone(), title: Some("Designing Data-Intensive Applications".into()), values: vec![(rating.clone(), None)], ..Default::default() }, PERSON).unwrap();
        assert_eq!(note.title.as_deref(), Some("Designing Data-Intensive Applications"));
        assert!(!values_of(&notes, &row.id).contains_key(&rating));
        let error = notes.update(UpdateNote { id: row.id.clone(), values: vec![(status, Some(serde_json::json!("nope")))], ..Default::default() }, PERSON).unwrap_err().to_string();
        assert!(error.contains("Reading"), "{error}");
    }

    #[test]
    fn set_values_refuses_an_unknown_property_option_or_type_and_writes_nothing() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        let seq = notes.read(&row.id).unwrap().seq;
        let unknown = notes.set_values(&row.id, &[("nope".into(), Some(serde_json::json!(1)))], PERSON).unwrap_err().to_string();
        assert!(unknown.contains("Rating") && unknown.contains("Status"), "the error names the valid properties: {unknown}");
        let option = notes.set_values(&row.id, &[(status.clone(), Some(serde_json::json!("nope")))], PERSON).unwrap_err().to_string();
        assert!(option.contains("Reading"), "the error names the valid options: {option}");
        assert!(notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!("five")))], PERSON).is_err());
        assert!(notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(null)))], PERSON).is_err());
        assert_eq!(notes.read(&row.id).unwrap().seq, seq, "a refused patch writes nothing");
        assert!(notes.set_values(&row.id, &[], PERSON).is_err());
        let loose = notes.capture("not a row", PERSON).unwrap();
        assert!(notes.set_values(&loose.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).is_err());
        assert!(notes.set_values(&id, &[(rating, Some(serde_json::json!(5)))], PERSON).is_err());
    }

    #[test]
    fn retyping_converts_every_row_in_one_attributed_write() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let rows: Vec<String> = [5, 4]
            .into_iter()
            .map(|value| {
                let row = notes.create(CreatePage { title: Some(format!("Book {value}")), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
                notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(value)))], PERSON).unwrap();
                row.id
            })
            .collect();
        let before: Vec<i64> = rows.iter().map(|row| notes.read(row).unwrap().seq).collect();
        let actor = agent_actor("claude", "retype");
        let updated = notes.update_schema(&id, &[SchemaOp::Retype { prop: rating.clone(), to: "text".into() }], &actor).unwrap();
        assert!(updated.warnings.is_empty());
        assert_eq!(notes.database(&id).unwrap().schema[&rating].kind, PropertyType::Text);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(notes.read(row).unwrap().seq, before[index] + 1, "exactly one write per row");
            assert_eq!(notes.history(row).unwrap().versions[0].actor, actor);
            assert_eq!(values_of(&notes, row)[&rating], serde_json::json!(if index == 0 { "5" } else { "4" }));
        }
        let file = std::fs::read_to_string(notes.read(&rows[0]).unwrap().note.path.unwrap()).unwrap();
        assert!(file.contains("Rating: \"5\""), "{file}");
    }

    #[test]
    fn a_schema_rename_and_an_option_add_merge() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let notes = Arc::new(notes);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let operations = [
            SchemaOp::Rename { prop: rating.clone(), name: "Score".into() },
            SchemaOp::AddOption { prop: status.clone(), name: "Paused".into(), color: None, group: None },
        ];
        let workers: Vec<_> = operations
            .into_iter()
            .map(|op| {
                let (notes, id, barrier) = (notes.clone(), id.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    notes.update_schema(&id, &[op], PERSON).map(|_| ())
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        let database = notes.database(&id).unwrap();
        assert_eq!(database.schema[&rating].name, "Score", "the rename survived the merge");
        assert!(database.schema[&status].options.iter().any(|option| option.name == "Paused"), "the option survived the merge");
    }

    #[test]
    fn the_values_index_survives_a_rebuild_from_the_log() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        let reading = notes.database(&id).unwrap().schema[&status].options[0].id.clone();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(4))), (status.clone(), Some(serde_json::json!(reading)))], PERSON).unwrap();
        let props: Vec<(String, String)> = notes
            .with_db(|db| {
                let mut statement = db.prepare("SELECT prop_id, type FROM database_props ORDER BY prop_id")?;
                let rows: Vec<(String, String)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .unwrap();
        let indexed = |row: &str| -> Vec<(String, i64)> {
            notes
                .with_db(|db| {
                    let mut statement = db.prepare("SELECT prop_id, item FROM page_values WHERE page_id=?1 ORDER BY prop_id, item")?;
                    let rows: Vec<(String, i64)> = statement.query_map(params![row], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
                    Ok(rows)
                })
                .unwrap()
        };
        let values = indexed(&row.id);
        notes.with_db(|db| { db.execute("DELETE FROM page_values", [])?; db.execute("DELETE FROM database_props", [])?; Ok(()) }).unwrap();
        notes.rebuild_index().unwrap();
        let rebuilt: Vec<(String, String)> = notes
            .with_db(|db| {
                let mut statement = db.prepare("SELECT prop_id, type FROM database_props ORDER BY prop_id")?;
                let rows: Vec<(String, String)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .unwrap();
        assert_eq!(rebuilt, props);
        assert_eq!(indexed(&row.id), values);
        assert_eq!(values.len(), 2);
        assert_eq!(values_of(&notes, &row.id)[&rating], serde_json::json!(4));
        assert_eq!(notes.database(&id).unwrap().schema.len(), 2);
    }

    #[test]
    fn views_save_replace_and_delete_against_the_live_schema() {
        use crate::notes::doc::{ViewDef, ViewPropertyDef, ViewType};
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let view = |name: &str| ViewDef {
            name: name.into(),
            kind: ViewType::Board,
            order: 10,
            filter: Some(serde_json::json!({"and": [{"property": "state", "op": "isNot", "value": "archived"}]})),
            sorts: vec![],
            group_by: None,
            date_property: None,
            properties: vec![ViewPropertyDef { id: rating.clone(), visible: true, width: Some(200) }],
        };
        let view_id = notes.put_view(&id, None, view("Board"), PERSON).unwrap();
        assert_eq!(notes.database(&id).unwrap().views[&view_id].name, "Board");
        notes.put_view(&id, Some(&view_id), view("By rating"), PERSON).unwrap();
        let saved = notes.database(&id).unwrap().views;
        assert_eq!(saved.len(), 2, "the seeded table plus one");
        assert_eq!(saved[&view_id].name, "By rating");
        assert!(notes.put_view(&id, None, ViewDef { name: "  ".into(), ..view("x") }, PERSON).is_err());
        assert!(notes.put_view(&id, None, ViewDef { properties: vec![ViewPropertyDef { id: "nope".into(), visible: true, width: None }], ..view("Bad") }, PERSON).is_err());
        notes.delete_view(&id, &view_id, PERSON).unwrap();
        assert!(!notes.database(&id).unwrap().views.contains_key(&view_id));
        assert!(notes.delete_view(&id, &view_id, PERSON).is_err());
    }

    #[test]
    fn renaming_an_option_rewrites_the_file_without_touching_the_doc() {
        let (_dir, notes) = service();
        let (id, _rating, status) = reading_list(&notes);
        let reading = notes.database(&id).unwrap().schema[&status].options[0].id.clone();
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(status.clone(), Some(serde_json::json!(reading)))], PERSON).unwrap();
        let page = notes.read(&row.id).unwrap();
        let path = page.note.path.clone().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("Status: \"Reading\""));
        notes.update_schema(&id, &[SchemaOp::RenameOption { prop: status, option: reading, name: "Reading now".into() }], PERSON).unwrap();
        crate::notes::projector::reconcile(&notes).unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("Status: \"Reading now\""), "{file}");
        assert_eq!(notes.read(&row.id).unwrap().seq, page.seq, "an option rename touches no row doc");
    }

    #[test]
    fn an_unclaimed_system_database_is_adopted_on_the_next_open() {
        let dir = tempfile::tempdir().unwrap();
        let check: crate::notes::service::ProjectCheck = std::sync::Arc::new(|_: &str| true);
        let notes = NotesService::open(dir.path(), check.clone()).unwrap();
        let id = notes.system_database().unwrap();
        // A crash between creating the page and recording it.
        notes.with_db(|db| Ok(db.execute("UPDATE spaces SET system_database_id=NULL", [])?)).unwrap();
        drop(notes);
        let reopened = NotesService::open(dir.path(), check).unwrap();
        assert_eq!(reopened.system_database().unwrap(), id, "the orphan is adopted, not duplicated");
        let databases: i64 = reopened.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages WHERE kind='database'", [], |row| row.get(0))?)).unwrap();
        assert_eq!(databases, 1);
    }

    #[test]
    fn a_retype_keeps_the_rules_an_add_has() {
        let (_dir, notes) = service();
        let (id, rating, status) = reading_list(&notes);
        assert!(notes.update_schema(&id, &[SchemaOp::Retype { prop: rating.clone(), to: "relation".into() }], PERSON).is_err(), "a relation cannot be retyped into being");
        // A status keeps groups; anything else drops them.
        notes.update_schema(&id, &[SchemaOp::Retype { prop: status.clone(), to: "select".into() }], PERSON).unwrap();
        let def = &notes.database(&id).unwrap().schema[&status];
        assert!(def.options.iter().all(|option| option.group.is_none()), "a select option has no status group");
        assert!(def.relation.is_none() && def.date_format.is_none());
        notes.update_schema(&id, &[SchemaOp::Retype { prop: status.clone(), to: "status".into() }], PERSON).unwrap();
        let def = &notes.database(&id).unwrap().schema[&status];
        assert!(def.options.iter().all(|option| option.group.is_some()), "a status option always has a group");
    }

    #[test]
    fn a_system_database_in_another_space_indexes_under_its_own_space() {
        let (dir, notes) = service();
        notes.with_db(|db| Ok(db.execute("INSERT INTO spaces(id, name, kind, created_at) VALUES('foreign', 'Other', 'team', 'then')", [])?)).unwrap();
        drop(notes);
        let check: crate::notes::service::ProjectCheck = std::sync::Arc::new(|_: &str| true);
        let notes = NotesService::open(dir.path(), check).unwrap();
        let foreign: String = notes
            .with_db(|db| Ok(db.query_row("SELECT system_database_id FROM spaces WHERE id='foreign'", [], |row| row.get::<_, String>(0))?))
            .unwrap();
        notes.update_schema(&foreign, &[SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        let spaces: Vec<String> = notes
            .with_db(|db| {
                let mut statement = db.prepare("SELECT DISTINCT space_id FROM database_props WHERE database_id=?1")?;
                let rows: Vec<String> = statement.query_map(params![foreign], |row| row.get(0))?.collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .unwrap();
        assert_eq!(spaces, vec!["foreign"], "one row per property, under the database's own space");
    }

    #[test]
    fn writing_a_database_keeps_its_rows_indexed() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        let indexed = |notes: &NotesService| -> i64 {
            notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM page_values WHERE database_id=?1", params![id], |row| row.get(0))?)).unwrap()
        };
        assert_eq!(indexed(&notes), 1);
        notes.update(UpdateNote { id: id.clone(), title: Some("Books".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(indexed(&notes), 1, "renaming a database must not empty its rows' values");
        let view = crate::notes::doc::ViewDef {
            name: "Board".into(),
            kind: crate::notes::doc::ViewType::Board,
            order: 9,
            filter: None,
            sorts: vec![],
            group_by: None,
            date_property: None,
            properties: vec![],
        };
        let view_id = notes.put_view(&id, None, view, PERSON).unwrap();
        assert_eq!(indexed(&notes), 1, "saving a view must not empty the rows' values");
        notes.delete_view(&id, &view_id, PERSON).unwrap();
        assert_eq!(indexed(&notes), 1);
    }

    #[test]
    fn the_system_database_is_not_a_page_a_person_can_reach() {
        let (_dir, notes) = service();
        let system = notes.system_database().unwrap();
        assert_eq!(notes.read(&system).unwrap().note.path, None, "the record has no file");
        let named = notes.create(CreatePage { title: Some("Notes".into()), ..Default::default() }, PERSON).unwrap();
        let linker = notes.create(CreatePage { title: Some("Linker".into()), content: Some("see [[Notes]]".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(notes.read(&named.id).unwrap().backlinks[0].page_id, linker.id, "[[Notes]] is the person's page");
        assert!(notes.list(&ListFilter { query: Some("Notes".into()), ..Default::default() }).unwrap().iter().all(|note| note.id != system));
        assert!(notes.delete(&system, PERSON).is_err());
        assert!(notes.update(UpdateNote { id: system.clone(), state: Some("archived".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.update(UpdateNote { id: system.clone(), title: Some("Renamed".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.edit(EditPage { id: system.clone(), text: Some("x".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.create(CreatePage { title: Some("Row".into()), parent_id: Some(system.clone()), ..Default::default() }, PERSON).is_err());
        let loose = notes.capture("move me", PERSON).unwrap();
        assert!(notes.update(UpdateNote { id: loose.id, parent_id: Some(system.clone()), ..Default::default() }, PERSON).is_err(), "a move under the record is refused too");
        assert!(notes.update_schema(&system, &[SchemaOp::Add(NewProperty { name: "Extra".into(), kind: "text".into(), ..Default::default() })], PERSON).is_ok(), "a person owns the views");
        assert!(notes.create(CreatePage { title: Some("Mine".into()), kind: Some("database".into()), scope: Some("space".into()), ..Default::default() }, PERSON).is_err());
    }

    #[test]
    fn a_database_cannot_become_a_row_of_another_database() {
        let (_dir, notes) = service();
        let first = reading_list(&notes).0;
        let second = reading_list(&notes).0;
        assert!(notes.update(UpdateNote { id: second, parent_id: Some(first), ..Default::default() }, PERSON).is_err());
    }

    #[test]
    fn concurrent_adds_of_one_name_leave_one_property() {
        let (_dir, notes) = service();
        let id = crate::notes::database::tests::database(&notes, "Books");
        let notes = Arc::new(notes);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (notes, id, barrier) = (notes.clone(), id.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    notes.update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).is_ok()
                })
            })
            .collect();
        let wins = workers.into_iter().map(|worker| worker.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(wins, 1, "one add wins the name, the other is refused");
        assert_eq!(notes.database(&id).unwrap().schema.len(), 1);
    }

    #[test]
    fn a_batch_cannot_change_the_same_property_twice() {
        let (_dir, notes) = service();
        let (id, rating, _) = reading_list(&notes);
        assert!(notes.update_schema(&id, &[SchemaOp::Retype { prop: rating.clone(), to: "text".into() }, SchemaOp::Delete { prop: rating.clone() }], PERSON).is_err());
        assert!(notes.update_schema(&id, &[SchemaOp::Rename { prop: rating.clone(), name: "Score".into() }, SchemaOp::AddOption { prop: rating, name: "x".into(), color: None, group: None }], PERSON).is_err());
    }

    #[test]
    fn a_deleted_database_leaves_no_index_after_a_rebuild() {
        let (_dir, notes) = service();
        let id = crate::notes::database::tests::database(&notes, "Gone");
        crate::notes::database::tests::property(&notes, &id, "Rating", PropertyType::Number);
        let props = |notes: &NotesService| -> i64 {
            notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM database_props WHERE database_id=?1", params![id], |row| row.get(0))?)).unwrap()
        };
        notes.delete(&id, PERSON).unwrap();
        assert_eq!(props(&notes), 0);
        // A deleted row leaves no values behind either.
        let other = crate::notes::database::tests::database(&notes, "Kept");
        let rating = crate::notes::database::tests::property(&notes, &other, "Rating", PropertyType::Number);
        let row = notes.create(CreatePage { title: Some("Row".into()), parent_id: Some(other.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating, Some(serde_json::json!(1)))], PERSON).unwrap();
        notes.delete(&row.id, PERSON).unwrap();
        notes.rebuild_index().unwrap();
        assert_eq!(props(&notes), 0, "a rebuild must not resurrect a deleted database's properties");
        let values: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM page_values WHERE page_id=?1", params![row.id], |r| r.get(0))?)).unwrap();
        assert_eq!(values, 0, "a rebuild must not resurrect a deleted row's values");
    }

    #[test]
    fn a_relation_names_rows_of_its_own_database() {
        let (_dir, notes) = service();
        let library = crate::notes::database::tests::database(&notes, "Library");
        let book = notes.create(CreatePage { title: Some("Book".into()), parent_id: Some(library.clone()), ..Default::default() }, PERSON).unwrap();
        let loose = notes.capture("not a row", PERSON).unwrap();
        let (id, _rating, _status) = reading_list(&notes);
        let added = notes.update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Related".into(), kind: "relation".into(), relation: Some(library), ..Default::default() })], PERSON).unwrap();
        let related = added.added[0].0.clone();
        let row = notes.create(CreatePage { title: Some("Row".into()), parent_id: Some(id), ..Default::default() }, PERSON).unwrap();
        assert!(notes.set_values(&row.id, &[(related.clone(), Some(serde_json::json!(["nope"])))], PERSON).is_err());
        assert!(notes.set_values(&row.id, &[(related.clone(), Some(serde_json::json!([loose.id])))], PERSON).is_err(), "a page outside the database is not a row");
        notes.set_values(&row.id, &[(related, Some(serde_json::json!([book.id])))], PERSON).unwrap();
    }

    #[test]
    fn adding_a_property_takes_the_value_the_file_already_had() {
        let (_dir, notes) = service();
        let id = crate::notes::database::tests::database(&notes, "Books");
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        let path = notes.read(&row.id).unwrap().note.path.unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\nPriority: \"urgent\"\n---\n")).unwrap();
        crate::notes::projector::import_changes(&notes).unwrap();
        let added = notes.update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Priority".into(), kind: "text".into(), ..Default::default() })], PERSON).unwrap();
        assert_eq!(values_of(&notes, &row.id)[&added.added[0].0], serde_json::json!("urgent"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("Priority: \"urgent\""));
    }

    #[test]
    fn retyping_again_repairs_values_of_the_old_type() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        // A retype that failed part way leaves a text value behind in a number column.
        notes.write(&row.id, PERSON, |live| live.edit_records(&RecordsPatch::default().put_value(&rating, Some(&serde_json::json!("five"))))).unwrap();
        assert_eq!(values_of(&notes, &row.id)[&rating], serde_json::json!("five"));
        let updated = notes.update_schema(&id, &[SchemaOp::Retype { prop: rating.clone(), to: "number".into() }], PERSON).unwrap();
        assert!(updated.warnings.iter().any(|warning| warning.contains("already")), "{updated:?}");
        assert!(!values_of(&notes, &row.id).contains_key(&rating), "a value the type cannot hold is cleared, not left invisible");
        // The other direction keeps what a conversion can carry.
        let added = notes.update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Note".into(), kind: "text".into(), ..Default::default() })], PERSON).unwrap();
        let note = added.added[0].0.clone();
        notes.write(&row.id, PERSON, |live| live.edit_records(&RecordsPatch::default().put_value(&note, Some(&serde_json::json!(5))))).unwrap();
        notes.update_schema(&id, &[SchemaOp::Retype { prop: note.clone(), to: "text".into() }], PERSON).unwrap();
        assert_eq!(values_of(&notes, &row.id)[&note], serde_json::json!("5"), "a stranded number becomes its text");
    }

    #[test]
    fn option_edits_on_one_property_fold_into_one_batch() {
        let (_dir, notes) = service();
        let (id, _rating, status) = reading_list(&notes);
        notes.update_schema(
            &id,
            &[
                SchemaOp::AddOption { prop: status.clone(), name: "Paused".into(), color: None, group: None },
                SchemaOp::AddOption { prop: status.clone(), name: "Dropped".into(), color: None, group: None },
            ],
            PERSON,
        )
        .unwrap();
        assert_eq!(notes.database(&id).unwrap().schema[&status].options.len(), 4);
    }

    #[test]
    fn a_clear_survives_a_dirty_file_and_a_later_outside_edit() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        // The file is dirty when the person clears the cell.
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\n---\n\ntyped in obsidian\n")).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), None)], PERSON).unwrap();
        crate::notes::projector::import_changes(&notes).unwrap();
        crate::notes::projector::reconcile(&notes).unwrap();
        assert!(!values_of(&notes, &row.id).contains_key(&rating));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("Rating:"), "the projection drops the stale line");
        // A later outside edit must not read the old line back in.
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("typed in obsidian", "typed again in obsidian")).unwrap();
        crate::notes::projector::import_changes(&notes).unwrap();
        assert!(!values_of(&notes, &row.id).contains_key(&rating), "the clear is not undone");
        assert!(notes.read(&row.id).unwrap().markdown.contains("typed again in obsidian"));
    }

    #[test]
    fn a_cleared_value_typed_back_into_the_file_imports() {
        // Cleared in the file (an accidental delete) and in Bridge: either way,
        // putting the same line back afterwards is an edit, not the stale copy.
        for in_bridge in [false, true] {
            let (_dir, notes) = service();
            let (id, rating, _status) = reading_list(&notes);
            let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
            notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
            let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
            if in_bridge {
                notes.set_values(&row.id, &[(rating.clone(), None)], PERSON).unwrap();
            } else {
                let file = std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, file.replace("Rating: 5\n", "")).unwrap();
                crate::notes::projector::import_changes(&notes).unwrap();
            }
            crate::notes::projector::reconcile(&notes).unwrap();
            assert!(!values_of(&notes, &row.id).contains_key(&rating));
            let file = std::fs::read_to_string(&path).unwrap();
            assert!(!file.contains("Rating:"), "{file}");
            std::fs::write(&path, file.replacen("\n---\n", "\nRating: 5\n---\n", 1)).unwrap();
            crate::notes::projector::import_changes(&notes).unwrap();
            assert_eq!(values_of(&notes, &row.id).get(&rating), Some(&serde_json::json!(5)), "in_bridge={in_bridge}: the line typed back in is the person's value");
            assert!(notes.with_db(|db| Ok(crate::notes::doc::load_page(db, &row.id, None)?.0.cleared())).unwrap().is_empty(), "the hold settles");
        }
    }

    #[test]
    fn re_adding_a_deleted_property_starts_empty() {
        let (_dir, notes) = service();
        let (id, rating, _status) = reading_list(&notes);
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        notes.update_schema(&id, &[SchemaOp::Delete { prop: rating.clone() }], PERSON).unwrap();
        crate::notes::projector::reconcile(&notes).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().contains("Rating: 5"), "a deleted column's line stays as a foreign key");
        let added = notes.update_schema(&id, &[SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        assert!(!values_of(&notes, &row.id).contains_key(&added.added[0].0), "the old line is not restored silently");
    }

    #[test]
    fn a_seeded_database_indexes_every_row() {
        let (_dir, notes) = service();
        let id = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap().id;
        crate::notes::database::tests::seed_rows(&notes, &id, 25);
        let indexed: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM page_values WHERE database_id=?1", params![id], |row| row.get(0))?)).unwrap();
        assert_eq!(indexed, 25);
        let rows = notes.with_db(|db| crate::notes::database::row_ids(db, &id)).unwrap();
        assert_eq!(rows.len(), 25);
    }
}
