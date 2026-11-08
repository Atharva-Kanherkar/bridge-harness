// A database as a month: six weeks from the Monday on or before the 1st, rows
// placed by the view's date property (a range on every day it covers, a
// datetime on its UTC day, as the planner reads it). Each month queries just
// its days; undated rows wait in a "No date" tray. Dropping on a day writes
// the new date, keeping a datetime's time and offset and a range's length.

import { useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { DatabaseRow } from "../../../types";
import type { ViewBodyProps } from "./viewBody";
import { composeFilter, readAll } from "./viewBody";
import { filterColumns } from "./viewModel";

const DAY_MS = 86_400_000;
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

const dayNumber = (day: string) => Date.parse(`${day}T00:00:00Z`) / DAY_MS;
const fromNumber = (value: number) => new Date(value * DAY_MS).toISOString().slice(0, 10);
export const addDays = (day: string, days: number) => fromNumber(dayNumber(day) + days);

/** The 42 days a month shows: from the Monday on or before its 1st. */
export function monthDays(month: string): string[] {
  const first = `${month}-01`;
  const weekday = (new Date(`${first}T00:00:00Z`).getUTCDay() + 6) % 7;
  const start = addDays(first, -weekday);
  return Array.from({ length: 42 }, (_, index) => addDays(start, index));
}

/** The day a stored date sits on: a day as written, a datetime's UTC day. */
export const placedDay = (raw: string) => raw.length === 10 ? raw : new Date(raw).toISOString().slice(0, 10);

/** A stored date moved `days` days, keeping a datetime's time and offset. */
export function shiftDate(raw: string, days: number): string {
  if (raw.length === 10) return addDays(raw, days);
  return `${addDays(raw.slice(0, 10), days)}${raw.slice(10)}`;
}

/** The value a date takes when its first day is dropped on `day`. */
export function droppedValue(value: unknown, day: string): { start: string; end?: string } {
  const date = value && typeof value === "object" ? value as { start?: string; end?: string } : null;
  if (!date?.start) return { start: day };
  const delta = dayNumber(day) - dayNumber(placedDay(date.start));
  return date.end ? { start: shiftDate(date.start, delta), end: shiftDate(date.end, delta) } : { start: shiftDate(date.start, delta) };
}

const localToday = () => {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
};

export function DatabaseCalendar({ database, view, query, run, version, onCommit, onOpenRow, today = localToday() }: ViewBodyProps & { today?: string }) {
  const dateColumn = useMemo(() => filterColumns(database).find(column => column.id === view.dateProperty && (column.type === "date" || column.type === "created" || column.type === "updated")), [database, view.dateProperty]);
  const [month, setMonth] = useState(today.slice(0, 7));
  const days = useMemo(() => monthDays(month), [month]);
  const [placed, setPlaced] = useState<DatabaseRow[]>([]);
  const [undated, setUndated] = useState<DatabaseRow[]>([]);
  const [dragging, setDragging] = useState<DatabaseRow | null>(null);
  const writable = dateColumn?.type === "date";

  const filterKey = JSON.stringify([query.filter, query.sorts]);
  useEffect(() => {
    if (!dateColumn) return;
    let alive = true;
    const first = days[0];
    const last = days[days.length - 1];
    const startOf = (row: DatabaseRow) => { const value = row.values[dateColumn.id] as { start?: string } | undefined; return value?.start ? placedDay(value.start) : null; };
    // Composed within the planner's limits when they allow, else the view's
    // own rows with the window applied here.
    const read = (conditions: { property: string; op: string; value?: unknown }[], limit: number, matches: (row: DatabaseRow) => boolean) => {
      const composed = composeFilter(query.filter, conditions);
      return composed !== null
        ? run({ filter: composed, sorts: query.sorts, limit }).then(result => result.rows)
        : readAll(run, { filter: query.filter ?? null, sorts: query.sorts }).then(rows => rows.filter(matches));
    };
    void read([{ property: dateColumn.id, op: "onOrAfter", value: first }, { property: dateColumn.id, op: "onOrBefore", value: last }], 500, row => { const start = startOf(row); return !!start && first <= start && start <= last; })
      .then(rows => { if (alive) setPlaced(rows); }, () => undefined);
    if (writable) void read([{ property: dateColumn.id, op: "isEmpty" }], 100, row => !startOf(row))
      .then(rows => { if (alive) setUndated(rows); }, () => undefined);
    return () => { alive = false; };
  }, [dateColumn, days, filterKey, version, run, writable]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!dateColumn) return <p role="status" className="m-0 rounded-lg bg-muted px-3 py-6 text-center text-[13px] text-muted-foreground">Add a date property to use a calendar</p>;

  const span = (row: DatabaseRow): [string, string] | null => {
    if (dateColumn.type !== "date") {
      const raw = dateColumn.type === "created" ? row.note.createdAt : row.note.updatedAt;
      return [placedDay(raw), placedDay(raw)];
    }
    const value = row.values[dateColumn.id] as { start?: string; end?: string } | undefined;
    if (!value?.start) return null;
    return [placedDay(value.start), placedDay(value.end ?? value.start)];
  };
  const onDay = (day: string) => placed.filter(row => { const range = span(row); return !!range && range[0] <= day && day <= range[1]; });
  const drop = async (day: string) => {
    const row = dragging;
    setDragging(null);
    if (!row || !writable) return;
    const value = droppedValue(row.values[dateColumn.id], day);
    // Shown on its new day at once; a refusal puts it back.
    const moved = { ...row, values: { ...row.values, [dateColumn.id]: value } };
    const before = { placed, undated };
    setPlaced(current => [...current.filter(item => item.note.id !== row.note.id), moved]);
    setUndated(current => current.filter(item => item.note.id !== row.note.id));
    if (!(await onCommit(row, dateColumn, value))) { setPlaced(before.placed); setUndated(before.undated); }
  };
  const [year, monthNumber] = month.split("-").map(Number);
  const shiftMonth = (by: number) => setMonth(new Date(Date.UTC(year, monthNumber - 1 + by, 1)).toISOString().slice(0, 7));
  const item = (row: DatabaseRow) => <button
    key={row.note.id}
    type="button"
    draggable={writable}
    onDragStart={event => { event.dataTransfer?.setData("text/plain", row.note.id); setDragging(row); }}
    onDragEnd={() => setDragging(null)}
    onClick={() => onOpenRow(row.note.id)}
    className="block w-full truncate rounded px-1 py-0.5 text-left text-[11.5px] text-foreground ring-1 ring-border transition-colors hover:bg-accent"
  >{row.note.title || "Untitled"}</button>;

  return <div aria-label="Calendar" className="flex gap-3">
    <div className="min-w-0 flex-1">
      <div className="mb-2 flex items-center gap-1">
        <h3 className="m-0 mr-auto text-[14px] font-semibold text-foreground">{MONTHS[monthNumber - 1]} {year}</h3>
        <button type="button" aria-label="Previous month" onClick={() => shiftMonth(-1)} className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ChevronLeft size={14} aria-hidden="true" /></button>
        <button type="button" onClick={() => setMonth(today.slice(0, 7))} className="h-7 rounded-md px-2 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Today</button>
        <button type="button" aria-label="Next month" onClick={() => shiftMonth(1)} className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ChevronRight size={14} aria-hidden="true" /></button>
      </div>
      <div role="grid" aria-label={`${MONTHS[monthNumber - 1]} ${year}`} className="grid grid-cols-7 overflow-hidden rounded-lg ring-1 ring-border">
        {WEEKDAYS.map(name => <div key={name} role="columnheader" className="border-b border-border px-1.5 py-1 text-[11px] text-muted-foreground">{name}</div>)}
        {days.map(day => <div
          key={day}
          role="gridcell"
          aria-label={day}
          onDragOver={event => { if (dragging) event.preventDefault(); }}
          onDrop={event => { event.preventDefault(); void drop(day); }}
          className={clsx("flex min-h-24 flex-col gap-0.5 border-b border-r border-border p-1 [&:nth-child(7n+7)]:border-r-0", day.slice(0, 7) !== month && "bg-muted/40")}
        >
          <span className={clsx("text-[11.5px] tabular-nums", day === today ? "font-semibold text-foreground" : "text-muted-foreground")}>{Number(day.slice(8))}</span>
          {onDay(day).map(item)}
        </div>)}
      </div>
    </div>
    {writable && <aside
      aria-label="No date"
      onDragOver={event => event.preventDefault()}
      className="w-44 shrink-0"
    >
      <h3 className="m-0 mb-2 flex h-7 items-center text-[12.5px] font-medium text-foreground">No date <span className="ml-1.5 font-normal text-muted-foreground tabular-nums">{undated.length}</span></h3>
      <div className="flex flex-col gap-1">{undated.length ? undated.map(item) : <p className="m-0 text-[12px] text-muted-foreground">Every row has a date.</p>}</div>
    </aside>}
  </div>;
}
