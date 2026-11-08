// A database as a board: one column per group of the view's grouping, in the
// planner's order, with its count. Moving a card to another column writes
// the grouped value (drag, or "Move to…" from the keyboard); the move shows at
// once and goes back if the write is refused. Cards follow the view's sorts.

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRightLeft, Plus } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { DatabaseGroup, DatabaseRow } from "../../../types";
import { Floating } from "./Floating";
import { cellValue, columnsOf, formatValue, type Column } from "./formatValue";
import type { ViewBodyProps } from "./viewBody";
import { composeFilter, readAll } from "./viewBody";
import { filterColumns, type Condition, type FieldColumn } from "./viewModel";

export const GROUP_LIMIT = 50;
const MOVABLE = new Set(["select", "status", "multiSelect", "checkbox", "kind", "state", "project"]);

/** The conditions that pick one group's rows, for its next page. */
export function groupConditions(column: FieldColumn, key: string): Condition[] {
  if (column.type === "checkbox") return [{ property: column.id, op: "is", value: key === "true" }];
  if (column.type === "multiSelect") return [key ? { property: column.id, op: "contains", value: key } : { property: column.id, op: "isEmpty" }];
  return [key ? { property: column.id, op: "is", value: key } : { property: column.id, op: "isEmpty" }];
}

/** Whether a row sits in group `key`, as the planner buckets it. */
export function inGroup(column: FieldColumn, key: string, row: DatabaseRow): boolean {
  const value = cellValue(column, row.note, row.values);
  if (column.type === "checkbox") return (value === true) === (key === "true");
  if (column.type === "multiSelect") return Array.isArray(value) && value.length ? value.includes(key) : key === "";
  return key ? value === key : value === null || value === undefined || value === "";
}

/** The value a card takes when moved from group `from` to group `to`. */
export function movedValue(column: FieldColumn, current: unknown, from: string, to: string): unknown {
  switch (column.type) {
    case "checkbox": return to === "true";
    case "multiSelect": {
      const list = (Array.isArray(current) ? current : []).map(String).filter(id => id !== from);
      if (to && !list.includes(to)) list.push(to);
      return list.length ? list : null;
    }
    default: return to || null;
  }
}

interface Move { row: DatabaseRow; from: string; to: string }

export function DatabaseBoard({ database, view, query, groups, run, host, onCommit, onNewRow, onOpenRow }: ViewBodyProps) {
  const grouping = useMemo(() => filterColumns(database).find(column => column.id === query.groupBy?.property), [database, query.groupBy]);
  const shown = useMemo(() => columnsOf(database, view).filter(column => column.visible && column.id !== "title" && column.id !== query.groupBy?.property), [database, view, query.groupBy]);
  const [moves, setMoves] = useState<Move[]>([]);
  const [extra, setExtra] = useState<Record<string, { rows: DatabaseRow[]; cursor: string | null }>>({});
  const [dragging, setDragging] = useState<{ row: DatabaseRow; from: string } | null>(null);
  // A fresh answer from the server supersedes local moves and pages.
  useEffect(() => { setMoves([]); setExtra({}); }, [groups]);

  if (!grouping || !groups) return <p role="status" className="m-0 rounded-lg bg-muted px-3 py-6 text-center text-[13px] text-muted-foreground">Choose a property to group by</p>;
  // A date grouping is a bucket of days, which a move cannot write.
  const movable = MOVABLE.has(grouping.type);

  const columns: (DatabaseGroup & { cards: DatabaseRow[] })[] = groups.map(group => {
    const base = [...group.rows, ...(extra[group.key]?.rows ?? [])].filter(row => !moves.some(move => move.row.note.id === row.note.id && move.from === group.key));
    const arrived = moves.filter(move => move.to === group.key).map(move => move.row);
    const cards = [...arrived, ...base.filter(row => !arrived.some(other => other.note.id === row.note.id))];
    const delta = arrived.length - moves.filter(move => move.from === group.key).length;
    return { ...group, count: group.count + delta, cards };
  });

  const move = async (row: DatabaseRow, from: string, to: string) => {
    if (from === to || !movable) return;
    const value = movedValue(grouping, cellValue(grouping, row.note, row.values), from, to);
    const entry: Move = { row, from, to };
    setMoves(current => [...current.filter(item => item.row.note.id !== row.note.id), entry]);
    const stored = await onCommit(row, grouping, value);
    if (!stored) setMoves(current => current.filter(item => item !== entry));
  };

  const loadMore = async (group: DatabaseGroup, have: number) => {
    const page = extra[group.key];
    const filter = composeFilter(query.filter, groupConditions(grouping, group.key));
    if (filter === null) {
      // No composed filter fits the planner's limits: the column's rows from
      // the view's own, all at once.
      const rows = (await readAll(run, { filter: query.filter ?? null, sorts: query.sorts })).filter(row => inGroup(grouping, group.key, row));
      setExtra(current => ({ ...current, [group.key]: { rows: rows.slice(group.rows.length), cursor: null } }));
      return;
    }
    const result = page?.cursor
      ? await run({ filter, sorts: query.sorts, limit: GROUP_LIMIT, cursor: page.cursor })
      : await run({ filter, sorts: query.sorts, limit: have + GROUP_LIMIT });
    const fresh = page?.cursor ? result.rows : result.rows.slice(group.rows.length);
    setExtra(current => ({ ...current, [group.key]: { rows: [...(current[group.key]?.rows ?? []), ...fresh], cursor: result.nextCursor ?? null } }));
  };

  return <div aria-label="Board" className="flex gap-3 overflow-x-auto pb-2">
    {columns.map(column => <section
      key={column.key || "none"}
      aria-label={`${column.label}, ${column.count}`}
      onDragOver={event => { if (dragging) event.preventDefault(); }}
      onDrop={event => { event.preventDefault(); if (dragging) void move(dragging.row, dragging.from, column.key); setDragging(null); }}
      className="flex w-64 shrink-0 flex-col rounded-xl bg-muted/50 p-1.5"
    >
      <header className="flex h-7 items-center gap-1.5 px-1 text-[12.5px]">
        <span className="min-w-0 truncate font-medium text-foreground">{column.label}</span>
        <span className="text-muted-foreground tabular-nums">{column.count}</span>
      </header>
      <ol className="m-0 flex list-none flex-col gap-1.5 p-0">
        {column.cards.map(row => <li key={row.note.id}>
          <Card
            row={row}
            columns={shown}
            host={host}
            movable={movable}
            targets={columns.filter(other => other.key !== column.key).map(other => ({ key: other.key, label: other.label }))}
            onOpen={() => onOpenRow(row.note.id)}
            onDragStart={() => setDragging({ row, from: column.key })}
            onDragEnd={() => setDragging(null)}
            onMove={to => void move(row, column.key, to)}
          />
        </li>)}
      </ol>
      {column.count > column.cards.length && <button type="button" onClick={() => void loadMore(column, column.cards.length)} className="mt-1 h-7 rounded-md text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Load more ({column.count - column.cards.length})</button>}
      {movable && <button type="button" aria-label={`New in ${column.label}`} onClick={() => void onNewRow({ column: grouping, value: movedValue(grouping, undefined, "", column.key) })} className="mt-1 inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Plus size={13} aria-hidden="true" />New</button>}
    </section>)}
  </div>;
}

function Card({ row, columns, host, movable, targets, onOpen, onDragStart, onDragEnd, onMove }: {
  row: DatabaseRow;
  columns: Column[];
  host: ViewBodyProps["host"];
  movable: boolean;
  targets: { key: string; label: string }[];
  onOpen: () => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onMove: (to: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  const anchor = useRef<HTMLDivElement>(null);
  const context = { rowTitle: host.rowTitle, workspaceTitle: (id: string) => host.workspaces.find(workspace => workspace.id === id)?.title };
  const lines = columns.map(column => [column, formatValue(column, cellValue(column, row.note, row.values), context)] as const).filter(([column, text]) => text && !(column.type === "checkbox" && text === "No"));
  return <div
    ref={anchor}
    draggable={movable}
    onDragStart={event => { event.dataTransfer?.setData("text/plain", row.note.id); onDragStart(); }}
    onDragEnd={onDragEnd}
    className="group relative rounded-lg bg-background p-2 shadow-sm ring-1 ring-border"
  >
    <button type="button" onClick={onOpen} className="block w-full min-w-0 text-left">
      <span className={clsx("block truncate text-[13px] font-medium", row.note.title ? "text-foreground" : "text-faint")}>{row.note.title || "Untitled"}</span>
      {lines.map(([column, text]) => <span key={column.id} className="mt-0.5 block truncate text-[11.5px] text-muted-foreground">{column.type === "checkbox" ? column.name : text}</span>)}
    </button>
    {movable && targets.length > 0 && <button type="button" aria-label={`Move ${row.note.title || "Untitled"} to…`} aria-expanded={menu} onClick={() => setMenu(current => !current)} className="absolute right-1 top-1 grid size-6 place-items-center rounded text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100">
      <ArrowRightLeft size={12} aria-hidden="true" />
    </button>}
    {menu && <Floating anchor={anchor} label="Move to" onClose={() => setMenu(false)} width="w-56">
      <div role="menu" aria-label="Move to">
        {targets.map(target => <button key={target.key || "none"} type="button" role="menuitem" onClick={() => { setMenu(false); onMove(target.key); }} className="flex min-h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-foreground transition-colors hover:bg-accent">{target.label}</button>)}
      </div>
    </Floating>}
  </div>;
}
