// What a view's filter, sorts and grouping can say, as #888's planner reads
// them: the ops each property takes, a fresh condition per property, and
// path edits on the filter tree (one group deep, at most 20 conditions).

import type { NoteDatabase, NoteProperty } from "../../../types";
import { TYPE_LABELS } from "./formatValue";

export interface Condition { property: string; op: string; value?: unknown }
export interface Group { join: "and" | "or"; items: (Condition | Group)[] }
/** `[]` is the top group, `[i]` an item in it, `[i, j]` an item in the group at `i`. */
export type Path = number[];

export const MAX_CONDITIONS = 20;
export const KIND_OPTIONS = ["note", "idea", "task", "question", "database"];
export const STATE_OPTIONS = ["inbox", "open", "doing", "done", "archived"];
export const WITHIN = [["pastWeek", "the past week"], ["nextWeek", "the next week"], ["thisWeek", "this week"], ["thisMonth", "this month"], ["pastMonth", "the past month"], ["nextMonth", "the next month"]] as const;

/** A property a view can name: a schema property or a page field. */
export interface FieldColumn { id: string; name: string; type: string; property?: NoteProperty }

const SYSTEM_FIELDS: [string, string][] = [["title", "Title"], ["kind", "Type"], ["state", "State"], ["tags", "Tags"], ["project", "Project"], ["chats", "Chats"], ["parent", "Parent"], ["created", "Created"], ["updated", "Updated"], ["createdBy", "Created by"]];

/** Every property a filter can name: the schema's, then the page fields. */
export function filterColumns(database: NoteDatabase): FieldColumn[] {
  const schema = [...database.properties].sort((a, b) => a.order - b.order).map(property => ({ id: property.id, name: property.name, type: property.type, property }));
  return [...schema, ...SYSTEM_FIELDS.map(([id, name]) => ({ id, name, type: id }))];
}

const FAMILY: Record<string, string> = {
  title: "text", text: "text", url: "text", createdBy: "text",
  number: "number",
  date: "date", created: "date", updated: "date",
  select: "choice", status: "choice", kind: "choice", state: "choice", project: "choice",
  multiSelect: "list", relation: "list", tags: "list",
  checkbox: "checkbox", chats: "chats", parent: "parent",
};
const OPS: Record<string, string[]> = {
  text: ["contains", "notContains", "equals", "notEquals", "startsWith", "isEmpty", "isNotEmpty"],
  number: ["eq", "ne", "lt", "lte", "gt", "gte", "isEmpty", "isNotEmpty"],
  date: ["before", "after", "onOrBefore", "onOrAfter", "equals", "isEmpty", "isNotEmpty", "within"],
  choice: ["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty"],
  list: ["contains", "notContains", "containsAny", "isEmpty", "isNotEmpty"],
  checkbox: ["is"],
  chats: ["isEmpty", "isNotEmpty"],
  parent: ["isEmpty", "isNotEmpty", "equals", "notEquals"],
};

export const familyOf = (type: string) => FAMILY[type] ?? "text";

/** The ops a property takes, as the planner does. */
export function opsFor(column: Pick<FieldColumn, "type">): string[] {
  const ops = OPS[familyOf(column.type)];
  return column.type === "status" ? [...ops, "inGroup"] : ops;
}

export const OP_LABELS: Record<string, string> = {
  contains: "contains", notContains: "does not contain", equals: "is", notEquals: "is not", startsWith: "starts with",
  isEmpty: "is empty", isNotEmpty: "is not empty",
  eq: "=", ne: "≠", lt: "<", lte: "≤", gt: ">", gte: "≥",
  before: "is before", after: "is after", onOrBefore: "is on or before", onOrAfter: "is on or after", within: "is within",
  is: "is", isNot: "is not", isAnyOf: "is any of", isNoneOf: "is none of", inGroup: "is in group", containsAny: "contains any of",
};

export const needsValue = (op: string) => op !== "isEmpty" && op !== "isNotEmpty";
const LIST_OPS = new Set(["isAnyOf", "isNoneOf", "containsAny"]);

/** The choices a choice-like column offers, as [value, label]. */
export function choicesOf(column: FieldColumn, workspaces: { id: string; title: string }[] = []): [string, string][] {
  if (column.property?.options) return column.property.options.map(option => [option.id, option.name]);
  if (column.type === "kind") return KIND_OPTIONS.map(kind => [kind, kind[0].toUpperCase() + kind.slice(1)]);
  if (column.type === "state") return STATE_OPTIONS.map(state => [state, state[0].toUpperCase() + state.slice(1)]);
  if (column.type === "project") return workspaces.map(workspace => [workspace.id, workspace.title]);
  return [];
}

/** An empty value of the shape `op` takes on `column`. */
export function emptyValue(column: FieldColumn, op: string): unknown {
  if (!needsValue(op)) return undefined;
  if (op === "within") return "pastWeek";
  if (op === "inGroup") return "todo";
  if (LIST_OPS.has(op)) return [];
  const family = familyOf(column.type);
  if (family === "number") return 0;
  if (family === "checkbox") return false;
  if (family === "choice") return choicesOf(column)[0]?.[0] ?? "";
  if (family === "list") return choicesOf(column)[0]?.[0] ?? "";
  return "";
}

/** A fresh condition on `column`: its first op, with an empty value. */
export function defaultCondition(column: FieldColumn): Condition {
  const op = opsFor(column)[0];
  const value = emptyValue(column, op);
  return value === undefined ? { property: column.id, op } : { property: column.id, op, value };
}

/** The condition with `op`, keeping a value that still fits. */
export function retarget(condition: Condition, column: FieldColumn, op: string): Condition {
  if (!needsValue(op)) return { property: condition.property, op };
  const before = condition.value;
  let value: unknown;
  if (LIST_OPS.has(op)) value = Array.isArray(before) ? before : before !== undefined && before !== "" ? [before] : [];
  else if (Array.isArray(before)) value = before[0] ?? emptyValue(column, op);
  else if (before === undefined || (op === "within") !== (condition.op === "within") || (op === "inGroup") !== (condition.op === "inGroup")) value = emptyValue(column, op);
  else value = before;
  return { property: condition.property, op, value };
}

export const isGroup = (item: Condition | Group): item is Group => "items" in item;

/** #888's JSON as a tree to edit; a bare condition or nothing is an `and` group. */
export function readFilter(filter: unknown): Group {
  const read = (node: unknown): Condition | Group | null => {
    if (!node || typeof node !== "object" || Array.isArray(node)) return null;
    const record = node as Record<string, unknown>;
    for (const join of ["and", "or"] as const) {
      if (Array.isArray(record[join])) return { join, items: (record[join] as unknown[]).map(read).filter((item): item is Condition | Group => item !== null) };
    }
    if (typeof record.property === "string" && typeof record.op === "string") return "value" in record ? { property: record.property, op: record.op, value: record.value } : { property: record.property, op: record.op };
    return null;
  };
  const top = read(filter);
  if (!top) return { join: "and", items: [] };
  return isGroup(top) ? top : { join: "and", items: [top] };
}

/** The tree as #888's JSON; an empty tree is no filter. */
export function writeFilter(group: Group): unknown {
  const write = (item: Condition | Group): unknown => isGroup(item) ? { [item.join]: item.items.map(write) } : item;
  return group.items.length ? write(group) : null;
}

export function countConditions(group: Group): number {
  return group.items.reduce((sum, item) => sum + (isGroup(item) ? countConditions(item) : 1), 0);
}
export const canAdd = (group: Group) => countConditions(group) < MAX_CONDITIONS;

const groupAt = (top: Group, path: Path): Group => path.length ? top.items[path[0]] as Group : top;

/** Add a condition to the top group (`[]`) or the group at `[i]`. */
export function addCondition(top: Group, path: Path, condition: Condition): Group {
  if (!canAdd(top)) return top;
  return update(top, path, group => ({ ...group, items: [...group.items, condition] }));
}

/** Add a group holding `condition` to the top group; groups nest one deep. */
export function addGroup(top: Group, condition: Condition): Group {
  if (!canAdd(top)) return top;
  return { ...top, items: [...top.items, { join: top.join === "and" ? "or" : "and", items: [condition] }] };
}

export function setCondition(top: Group, path: Path, condition: Condition): Group {
  const parent = path.slice(0, -1);
  const index = path[path.length - 1];
  return update(top, parent, group => ({ ...group, items: group.items.map((item, at) => at === index ? condition : item) }));
}

/** Remove the item at `path`; a nested group left empty goes too. */
export function removeAt(top: Group, path: Path): Group {
  const parent = path.slice(0, -1);
  const index = path[path.length - 1];
  const next = update(top, parent, group => ({ ...group, items: group.items.filter((_, at) => at !== index) }));
  return { ...next, items: next.items.filter(item => !isGroup(item) || item.items.length > 0) };
}

export function setJoin(top: Group, path: Path, join: "and" | "or"): Group {
  return update(top, path, group => ({ ...group, join }));
}

function update(top: Group, path: Path, change: (group: Group) => Group): Group {
  if (!path.length) return change(top);
  const target = groupAt(top, path);
  if (!target || !isGroup(target)) return top;
  return { ...top, items: top.items.map((item, at) => at === path[0] ? change(target) : item) };
}

/** Whether a condition is complete enough to send: a number must be finite. */
export function isComplete(condition: Condition, column: FieldColumn | undefined): boolean {
  if (!column || !needsValue(condition.op)) return !!column;
  if (familyOf(column.type) === "number") return typeof condition.value === "number" && Number.isFinite(condition.value);
  return condition.value !== undefined;
}

/** The tree with incomplete conditions left out, for sending. */
export function completeOnly(top: Group, columns: FieldColumn[]): Group {
  const byId = new Map(columns.map(column => [column.id, column]));
  const keep = (group: Group): Group => ({
    ...group,
    items: group.items.flatMap((item): (Condition | Group)[] => {
      if (isGroup(item)) { const kept = keep(item); return kept.items.length ? [kept] : []; }
      return isComplete(item, byId.get(item.property)) ? [item] : [];
    }),
  });
  return keep(top);
}

const GROUPABLE = new Set(["select", "status", "multiSelect", "checkbox", "date", "kind", "state", "project"]);
const UNSORTABLE = new Set(["multiSelect", "relation", "tags", "chats", "parent"]);

export const groupableColumns = (database: NoteDatabase) => filterColumns(database).filter(column => GROUPABLE.has(column.type));
export const sortableColumns = (database: NoteDatabase) => filterColumns(database).filter(column => !UNSORTABLE.has(column.type));
export const columnName = (columns: FieldColumn[], id: string) => columns.find(column => column.id === id)?.name ?? TYPE_LABELS[id] ?? id;
