// What a database cell shows, which columns a table view draws and how wide,
// and what a retype keeps. Pure, so every view type (#890's board, calendar
// and list too) draws a value the same way.

import type { Note, NoteDatabase, NoteDatabaseView, NoteProperty } from "../../../types";

export const PROPERTY_TYPES = ["text", "number", "select", "multiSelect", "status", "date", "checkbox", "url", "relation"] as const;
export const OPTION_TYPES = ["select", "multiSelect", "status"];
export const TYPE_LABELS: Record<string, string> = {
  text: "Text", number: "Number", select: "Select", multiSelect: "Multi-select", status: "Status", date: "Date",
  checkbox: "Checkbox", url: "URL", relation: "Relation", title: "Title",
  project: "Project", chats: "Chats", created: "Created", updated: "Updated",
};
export const STATUS_GROUPS: [string, string][] = [["todo", "To do"], ["inProgress", "In progress"], ["done", "Done"]];

/** The page fields a view can show as columns. Only those a view lists appear. */
export const SYSTEM_COLUMNS = ["project", "chats", "created", "updated"] as const;

export interface Column {
  /** A property id, `title`, or a system column id. */
  id: string;
  name: string;
  type: string;
  /** The schema property; absent for `title` and system columns. */
  property?: NoteProperty;
  width: number;
  visible: boolean;
}

export const MIN_WIDTH = 80;
export const MAX_WIDTH = 640;
const DEFAULT_WIDTHS: Record<string, number> = { title: 280, text: 200, url: 200, number: 120, checkbox: 96, date: 160, select: 180, status: 180, multiSelect: 180, relation: 180 };

export const clampWidth = (width: number) => Math.round(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, width)));
export const defaultWidth = (type: string) => DEFAULT_WIDTHS[type] ?? 160;
/** A width dragged `delta` px from `start`, inside the bounds. */
export const nextWidth = (start: number, delta: number) => clampWidth(start + delta);

/** The table's columns: `title`, then the view's entries in their order, then
 *  every schema property the view does not list, in schema order. */
export function columnsOf(database: NoteDatabase, view: NoteDatabaseView | undefined): Column[] {
  const schema = [...database.properties].sort((a, b) => a.order - b.order);
  const byId = new Map(schema.map(property => [property.id, property]));
  const entries = view?.properties ?? [];
  const titleEntry = entries.find(entry => entry.id === "title");
  const columns: Column[] = [{ id: "title", name: "Title", type: "title", width: clampWidth(titleEntry?.width ?? defaultWidth("title")), visible: true }];
  const seen = new Set<string>(["title"]);
  for (const entry of entries) {
    if (seen.has(entry.id)) continue;
    const property = byId.get(entry.id);
    const system = (SYSTEM_COLUMNS as readonly string[]).includes(entry.id);
    if (!property && !system) continue;
    seen.add(entry.id);
    const type = property?.type ?? entry.id;
    columns.push({ id: entry.id, name: property?.name ?? TYPE_LABELS[entry.id], type, property, width: clampWidth(entry.width ?? defaultWidth(type)), visible: entry.visible });
  }
  for (const property of schema) {
    if (seen.has(property.id)) continue;
    columns.push({ id: property.id, name: property.name, type: property.type, property, width: defaultWidth(property.type), visible: true });
  }
  return columns;
}

/** A column's value on a row: a cell, or the page field a system column shows. */
export function cellValue(column: Pick<Column, "id" | "property">, note: Note, values: Record<string, unknown>): unknown {
  if (column.property) return values[column.id];
  switch (column.id) {
    case "title": return note.title ?? null;
    case "project": return note.workspaceId ?? null;
    case "chats": return note.sessionIds;
    case "created": return note.createdAt;
    case "updated": return note.updatedAt;
    default: return undefined;
  }
}

export interface FormatContext {
  /** A related row's title, when the view knows it. */
  rowTitle?: (id: string) => string | null | undefined;
  workspaceTitle?: (id: string) => string | undefined;
  /** For tests; the device's zone otherwise. */
  timeZone?: string;
}

const DAY = /^\d{4}-\d{2}-\d{2}$/;
const numberFormat = new Intl.NumberFormat("en-US", { maximumFractionDigits: 6 });
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

/** A stored date or datetime as the page reads it: "Nov 12, 2026", plus
 *  ", 3:30 PM" in the device's zone for a datetime. */
export function formatDate(raw: string, timeZone?: string): string {
  if (DAY.test(raw)) {
    const time = Date.parse(`${raw}T00:00:00Z`);
    return Number.isNaN(time) ? "" : new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" }).format(time);
  }
  const time = Date.parse(raw);
  return Number.isNaN(time) ? "" : new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit", timeZone }).format(time);
}

const optionName = (property: NoteProperty | undefined, id: unknown) => typeof id === "string" ? property?.options?.find(option => option.id === id)?.name ?? "" : "";

/** The text a cell shows; `""` when it is empty. */
export function formatValue(column: Pick<Column, "type" | "property">, value: unknown, context: FormatContext = {}): string {
  if (column.type === "checkbox") return value === true ? "Yes" : "No";
  if (value === null || value === undefined) return "";
  switch (column.type) {
    case "title": case "text": case "url": return typeof value === "string" ? value : "";
    case "number": return typeof value === "number" && Number.isFinite(value) ? numberFormat.format(value) : "";
    case "select": case "status": return optionName(column.property, value);
    case "multiSelect": return Array.isArray(value) ? value.map(id => optionName(column.property, id)).filter(Boolean).join(", ") : "";
    case "date": case "created": case "updated": {
      if (typeof value === "string") return formatDate(value, context.timeZone);
      const date = value as { start?: unknown; end?: unknown };
      if (typeof date?.start !== "string") return "";
      const start = formatDate(date.start, context.timeZone);
      return typeof date.end === "string" ? `${start} → ${formatDate(date.end, context.timeZone)}` : start;
    }
    case "relation": {
      if (!Array.isArray(value) || !value.length) return "";
      const titles: string[] = [];
      let unknown = 0;
      for (const id of value) {
        const title = typeof id === "string" ? context.rowTitle?.(id) : undefined;
        if (title === undefined) unknown += 1;
        else titles.push(title?.trim() || "Untitled");
      }
      if (!titles.length) return plural(unknown, "page");
      return unknown ? `${titles.join(", ")} and ${plural(unknown, "more page")}` : titles.join(", ");
    }
    case "project": return typeof value === "string" ? context.workspaceTitle?.(value) ?? "" : "";
    case "chats": return Array.isArray(value) && value.length ? plural(value.length, "chat") : "";
    default: return "";
  }
}

/** What a retype keeps, as the core converts (#887). */
export function retypeNote(from: string, to: string): string {
  if (from === to) return "Keeps every value.";
  const pair = `${from}>${to}`;
  if (["number>text", "checkbox>text", "select>text", "status>text", "select>multiSelect"].includes(pair)) return "Keeps every value.";
  if (pair === "text>number") return "Keeps values that read as numbers and clears the rest.";
  if (pair === "text>checkbox") return "Keeps true and false and clears the rest.";
  if (pair === "text>select" || pair === "text>status") return "Keeps every value, adding an option for each one.";
  return "Clears every value.";
}
