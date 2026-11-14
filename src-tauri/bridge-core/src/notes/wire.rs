//! Database records on the wire: core defs keyed by id become wire structs
//! that carry their id, and wire inputs become the service's ops.

use bridge_protocol::messages as wire;

use super::database::DatabaseDoc;
use super::doc::{GroupByDef, PropertyDef, SortDef, ViewDef, ViewPropertyDef, ViewType};
use super::query::{Group, QueryResult, Row};
use super::service::{invalid, NewOption, NewProperty, OptionInput, SchemaOp};
use crate::BridgeError;

pub fn property(id: &str, def: &PropertyDef) -> wire::NoteProperty {
    wire::NoteProperty {
        id: id.to_owned(),
        name: def.name.clone(),
        kind: def.kind.as_str().to_owned(),
        order: def.order,
        options: def
            .options
            .iter()
            .map(|option| wire::NoteOption { id: option.id.clone(), name: option.name.clone(), color: option.color.clone(), group: option.group.clone() })
            .collect(),
        relation: def.relation.as_ref().map(|relation| wire::NoteRelation { database_id: relation.database_id.clone() }),
        date_format: def.date_format.clone(),
    }
}

pub fn view(id: &str, def: &ViewDef) -> wire::NoteDatabaseView {
    wire::NoteDatabaseView {
        id: id.to_owned(),
        name: def.name.clone(),
        kind: def.kind.as_str().to_owned(),
        order: def.order,
        filter: def.filter.clone(),
        sorts: def.sorts.iter().map(|sort| wire::NoteSort { property: sort.property.clone(), direction: sort.direction.clone() }).collect(),
        group_by: def.group_by.as_ref().map(|group| wire::NoteGroupBy { property: group.property.clone(), date_bucket: group.date_bucket.clone() }),
        date_property: def.date_property.clone(),
        properties: def.properties.iter().map(|property| wire::NoteViewProperty { id: property.id.clone(), visible: property.visible, width: property.width }).collect(),
    }
}

/// A database with its properties and views in display order.
pub fn database(doc: &DatabaseDoc) -> wire::NoteDatabase {
    let mut views: Vec<(&String, &ViewDef)> = doc.views.iter().collect();
    views.sort_by_key(|(id, view)| (view.order, id.as_str()));
    wire::NoteDatabase {
        id: doc.id.clone(),
        scope: doc.scope.clone(),
        properties: doc.ordered().into_iter().map(|(id, def)| property(id, def)).collect(),
        views: views.into_iter().map(|(id, def)| view(id, def)).collect(),
    }
}

fn option_inputs(options: &[wire::NoteOptionInput]) -> Vec<OptionInput> {
    options.iter().map(|option| OptionInput { id: option.id.clone(), name: option.name.clone(), color: option.color.clone(), group: option.group.clone() }).collect()
}

pub fn new_property(input: &wire::NotePropertyInput) -> Result<NewProperty, BridgeError> {
    let options = input.options.as_deref().unwrap_or_default();
    if let Some(option) = options.iter().find(|option| option.id.is_some()) {
        return Err(invalid(format!("a new property's option {} cannot carry an id", option.name)));
    }
    Ok(NewProperty {
        name: input.name.clone(),
        kind: input.kind.clone(),
        options: options.iter().map(|option| NewOption { name: option.name.clone(), color: option.color.clone(), group: option.group.clone() }).collect(),
        relation: input.relation.clone(),
        date_format: input.date_format.clone(),
    })
}

pub fn schema_op(change: &wire::NoteSchemaChange) -> Result<SchemaOp, BridgeError> {
    Ok(match change {
        wire::NoteSchemaChange::Add { property } => SchemaOp::Add(new_property(property)?),
        wire::NoteSchemaChange::Rename { property_id, name } => SchemaOp::Rename { prop: property_id.clone(), name: name.clone() },
        wire::NoteSchemaChange::Retype { property_id, kind } => SchemaOp::Retype { prop: property_id.clone(), to: kind.clone() },
        wire::NoteSchemaChange::Delete { property_id } => SchemaOp::Delete { prop: property_id.clone() },
        wire::NoteSchemaChange::SetOptions { property_id, options, base } => SchemaOp::SetOptions { prop: property_id.clone(), options: option_inputs(options), base: base.clone() },
        wire::NoteSchemaChange::Reorder { property_ids } => SchemaOp::Reorder { props: property_ids.clone() },
    })
}

/// A view input as the def it saves, with the id it replaces (if any). A new
/// view without an order goes last.
pub fn view_input(input: &wire::NoteViewInput, existing: Option<&DatabaseDoc>) -> Result<(Option<String>, ViewDef), BridgeError> {
    let kind = ViewType::parse(&input.kind).ok_or_else(|| invalid(format!("a view is table, board, calendar or list, not {}", input.kind)))?;
    let order = input.order.unwrap_or_else(|| {
        let replaced = input.id.as_ref().and_then(|id| existing.and_then(|doc| doc.views.get(id))).map(|view| view.order);
        replaced.unwrap_or_else(|| existing.and_then(|doc| doc.views.values().map(|view| view.order).max()).map_or(0, |max| max + 1))
    });
    Ok((
        input.id.clone(),
        ViewDef {
            name: input.name.clone(),
            kind,
            order,
            filter: input.filter.clone(),
            sorts: input.sorts.clone().unwrap_or_default().into_iter().map(|sort| SortDef { property: sort.property, direction: sort.direction }).collect(),
            group_by: input.group_by.clone().map(|group| GroupByDef { property: group.property, date_bucket: group.date_bucket }),
            date_property: input.date_property.clone(),
            properties: input.properties.clone().unwrap_or_default().into_iter().map(|property| ViewPropertyDef { id: property.id, visible: property.visible, width: property.width }).collect(),
        },
    ))
}

pub fn row(row: Row) -> wire::DatabaseRow {
    wire::DatabaseRow { note: row.note, values: row.values }
}

fn group(group: Group) -> wire::DatabaseGroup {
    wire::DatabaseGroup { key: group.key, label: group.label, count: group.count as u32, rows: group.rows.into_iter().map(row).collect() }
}

pub fn query_result(result: QueryResult) -> wire::DatabaseQueryResult {
    wire::DatabaseQueryResult {
        rows: result.rows.into_iter().map(row).collect(),
        groups: result.groups.map(|groups| groups.into_iter().map(group).collect()),
        total: result.total as u32,
        next_cursor: result.next_cursor,
    }
}

pub fn query_spec(params: &wire::QueryDatabaseParams) -> super::query::QuerySpec {
    super::query::QuerySpec {
        view_id: params.view_id.clone(),
        filter: params.filter.clone(),
        constraints: Vec::new(),
        sorts: params.sorts.clone().map(|sorts| sorts.into_iter().map(|sort| SortDef { property: sort.property, direction: sort.direction }).collect()),
        group_by: params.group_by.clone().map(|group| GroupByDef { property: group.property, date_bucket: group.date_bucket }),
        group_limit: params.group_limit.map(|limit| limit as usize),
        query: params.query.clone(),
        today: params.today.clone(),
        limit: params.limit.map(|limit| limit as usize),
        cursor: params.cursor.clone(),
        notes_only: params.notes_only,
    }
}
