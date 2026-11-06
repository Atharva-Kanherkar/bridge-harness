// A database as a list: one line per row, its title and the view's visible
// properties as compact text. Grouped, rows sit in collapsible sections;
// ungrouped, the lines are windowed like the table's.

import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { ChevronRight } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { DatabaseRow } from "../../../types";
import { ROW_HEIGHT, windowOf } from "./DatabaseTable";
import { cellValue, columnsOf, formatValue, type Column } from "./formatValue";
import type { ViewBodyProps } from "./viewBody";

export function DatabaseList({ database, view, query, rows, groups, host, onOpenRow }: ViewBodyProps) {
  const shown = useMemo(() => columnsOf(database, view).filter(column => column.visible && column.id !== "title"), [database, view]);
  const line = (row: DatabaseRow) => <Line key={row.note.id} row={row} columns={shown} host={host} onOpen={() => onOpenRow(row.note.id)} />;
  if (query.groupBy && groups) {
    return <div aria-label="List" className="flex flex-col gap-2">
      {groups.map(group => <Section key={group.key} label={group.label} count={group.count} shown={group.rows.length}>{group.rows.map(line)}</Section>)}
    </div>;
  }
  return <Windowed rows={rows} render={line} />;
}

function Section({ label, count, shown, children }: { label: string; count: number; shown: number; children: React.ReactNode }) {
  const [open, setOpen] = useState(true);
  return <section aria-label={label}>
    <button type="button" aria-expanded={open} onClick={() => setOpen(current => !current)} className="flex h-8 w-full items-center gap-1.5 rounded-md px-1 text-left text-[13px] font-medium text-foreground transition-colors hover:bg-accent">
      <ChevronRight size={14} aria-hidden="true" className={clsx("text-muted-foreground transition-transform", open && "rotate-90")} />
      {label}<span className="text-[12px] font-normal text-muted-foreground tabular-nums">{count}</span>
    </button>
    {open && <div role="list" className="mt-0.5 flex flex-col">{children}</div>}
    {open && count > shown && <p className="m-0 px-2 pt-1 text-[12px] text-muted-foreground">Showing {shown} of {count}. Filter the view to narrow it.</p>}
  </section>;
}

function Windowed({ rows, render }: { rows: DatabaseRow[]; render: (row: DatabaseRow) => React.ReactNode }) {
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);
  useLayoutEffect(() => {
    const box = scroller.current;
    if (!box) return;
    const measure = () => setHeight(box.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    return () => observer.disconnect();
  }, []);
  const [start, end] = windowOf(rows.length, scrollTop, height);
  if (!rows.length) return <p className="m-0 px-1 py-4 text-[13px] text-muted-foreground">No rows yet</p>;
  return <div ref={scroller} aria-label="List" onScroll={event => setScrollTop(event.currentTarget.scrollTop)} className="max-h-[70vh] overflow-y-auto">
    <div role="list">
      {start > 0 && <div aria-hidden="true" style={{ height: start * ROW_HEIGHT }} />}
      {rows.slice(start, end).map(render)}
      {end < rows.length && <div aria-hidden="true" style={{ height: (rows.length - end) * ROW_HEIGHT }} />}
    </div>
  </div>;
}

function Line({ row, columns, host, onOpen }: { row: DatabaseRow; columns: Column[]; host: ViewBodyProps["host"]; onOpen: () => void }) {
  const context = { rowTitle: host.rowTitle, workspaceTitle: (id: string) => host.workspaces.find(workspace => workspace.id === id)?.title };
  const chips = columns.map(column => [column, formatValue(column, cellValue(column, row.note, row.values), context)] as const).filter(([column, text]) => text && !(column.type === "checkbox" && text === "No"));
  return <div role="listitem" className="h-9">
    <button type="button" onClick={onOpen} className="flex h-9 w-full min-w-0 items-center gap-3 rounded-md px-2 text-left transition-colors hover:bg-accent">
      <span className={clsx("min-w-0 shrink truncate text-[13.5px] font-medium", row.note.title ? "text-foreground" : "text-faint")}>{row.note.title || "Untitled"}</span>
      <span className="flex min-w-0 flex-1 items-center justify-end gap-1.5 overflow-hidden">
        {chips.map(([column, text]) => <span key={column.id} title={`${column.name}: ${text}`} className="max-w-48 shrink-0 truncate rounded-md bg-muted px-1.5 py-0.5 text-[11.5px] text-muted-foreground">{column.type === "checkbox" ? column.name : text}</span>)}
      </span>
    </button>
  </div>;
}
