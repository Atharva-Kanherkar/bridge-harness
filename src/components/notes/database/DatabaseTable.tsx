// A database as a table: a sticky header, the title pinned at the left, one
// row per page in the order the query returned, and one selected cell that
// the keyboard moves and edits.
//
// Rows are windowed from the scroll container's own `scrollTop` (an
// IntersectionObserver's rootMargin is dead in nested scrollers in WKWebView),
// so a thousand rows draw a few dozen. The table never sorts or filters: the
// order is the server's.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowUpRight, Calendar, CheckSquare, CircleDashed, Clock, Folder, Hash, Link2, List, MessageSquare, Plus, Tags, Text as TextIcon, Type } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { DatabaseRow, NoteDatabase, NoteDatabaseView } from "../../../types";
import { Floating } from "./Floating";
import { PropertyCell, type CellHost, type CloseReason } from "./PropertyCell";
import { AddProperty, ColumnMenu, type ColumnActions } from "./SchemaMenu";
import { SYSTEM_COLUMNS, TYPE_LABELS, cellValue, columnsOf, defaultWidth, nextWidth, type Column } from "./formatValue";

export const ROW_HEIGHT = 36;
export const OVERSCAN = 10;
const ADD_WIDTH = 44;

export interface TableActions extends ColumnActions {
  resize: (column: Column, width: number) => void;
  show: (column: Column) => void;
  addProperty: (name: string, type: string) => void;
}

export interface DatabaseTableProps {
  database: NoteDatabase;
  view: NoteDatabaseView | undefined;
  rows: DatabaseRow[];
  host: CellHost;
  actions: TableActions;
  onCommit: (row: DatabaseRow, column: Column, value: unknown) => void;
  /** Make a row; resolves to its id once made. */
  onNewRow: () => Promise<string | null>;
  onOpenRow: (id: string) => void;
  /** A bounded frame (an embed) rather than a page's table. */
  bounded?: boolean;
}

const ICONS: Record<string, typeof TextIcon> = {
  title: Type, text: TextIcon, number: Hash, select: CircleDashed, status: CircleDashed, multiSelect: Tags, date: Calendar,
  checkbox: CheckSquare, url: Link2, relation: ArrowUpRight, project: Folder, chats: MessageSquare, created: Clock, updated: Clock,
};
export const typeIcon = (type: string, size = 13) => {
  const Icon = ICONS[type] ?? List;
  return <Icon size={size} strokeWidth={1.75} aria-hidden="true" />;
};

const EDITABLE_BY_TYPING = new Set(["title", "text", "number", "url"]);
// No editor at all. Chats is read-only too, but opens its links.
const READ_ONLY = new Set(["created", "updated"]);

/** The rows to draw for a scroll position: the visible ones plus `OVERSCAN`
 *  on each side. */
export function windowOf(count: number, scrollTop: number, height: number): [number, number] {
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(count, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
  return [start, Math.max(start, end)];
}

export function DatabaseTable({ database, view, rows, host, actions, onCommit, onNewRow, onOpenRow, bounded = false }: DatabaseTableProps) {
  const columns = useMemo(() => columnsOf(database, view), [database, view]);
  const visible = useMemo(() => columns.filter(column => column.visible), [columns]);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const widthOf = (column: Column) => widths[column.id] ?? column.width;
  const scroller = useRef<HTMLDivElement>(null);
  const grid = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);
  // The selected cell names its row and its column by id, so a refresh that
  // reorders rows, or a column hidden or moved beside it, never moves it (or
  // an open editor) to another cell.
  const [focus, setFocus] = useState<{ id: string; column: string } | null>(null);
  const selected = useMemo(() => {
    if (!focus) return null;
    const row = rows.findIndex(candidate => candidate.note.id === focus.id);
    const col = visible.findIndex(column => column.id === focus.column);
    return row < 0 || col < 0 ? null : { row, col };
  }, [focus, rows, visible]);
  const [editing, setEditing] = useState<{ seed?: string } | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [madeId, setMadeId] = useState<string | null>(null);
  // A width dragged here holds until the saved view brings its own.
  useEffect(() => setWidths({}), [view]);

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

  // The selection stays inside the table as rows and columns come and go.
  useEffect(() => {
    if (focus && (!visible.some(column => column.id === focus.column) || !rows.some(row => row.note.id === focus.id))) { setFocus(null); setEditing(null); }
  }, [focus, rows, visible]);


  const [start, end] = windowOf(rows.length, scrollTop, height);
  const totalWidth = visible.reduce((sum, column) => sum + widthOf(column), 0) + ADD_WIDTH;

  /** Bring a row into the scrolled window. */
  const reveal = useCallback((row: number) => {
    const box = scroller.current;
    if (!box) return;
    const top = row * ROW_HEIGHT;
    const header = ROW_HEIGHT;
    if (top < box.scrollTop) box.scrollTop = top;
    else if (top + ROW_HEIGHT > box.scrollTop + box.clientHeight - header) box.scrollTop = top + ROW_HEIGHT - box.clientHeight + header;
    setScrollTop(box.scrollTop);
  }, []);

  // A row made by `+ New` is scrolled into the window, selected and its title
  // opened, once the query returns it.
  useEffect(() => {
    if (!madeId) return;
    const index = rows.findIndex(row => row.note.id === madeId);
    if (index < 0) return;
    setMadeId(null);
    reveal(index);
    setFocus({ id: madeId, column: "title" });
    setEditing({});
  }, [madeId, rows, reveal]);

  const select = (row: number, col: number) => {
    setFocus({ id: rows[row].note.id, column: visible[col].id });
    setEditing(null);
    reveal(row);
  };
  const toggle = (row: DatabaseRow, column: Column) => onCommit(row, column, !(cellValue(column, row.note, row.values) === true));
  const openEditor = (row: number, col: number, seed?: string) => {
    const column = visible[col];
    if (!column || READ_ONLY.has(column.type)) return;
    if (column.type === "checkbox") { toggle(rows[row], column); return; }
    setEditing(seed === undefined ? {} : { seed });
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    // Only the grid's own focus navigates: a header button, a cell's Open
    // button or an editor keeps its keys.
    if (event.target !== event.currentTarget) return;
    if (editing || !rows.length || !visible.length) return;
    const at = selected ?? { row: 0, col: 0 };
    const lastRow = rows.length - 1;
    const lastCol = visible.length - 1;
    const move = (row: number, col: number) => { event.preventDefault(); select(Math.max(0, Math.min(lastRow, row)), Math.max(0, Math.min(lastCol, col))); };
    if (!selected && ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key)) { move(0, 0); return; }
    switch (event.key) {
      case "ArrowUp": move(at.row - 1, at.col); return;
      case "ArrowDown": move(at.row + 1, at.col); return;
      case "ArrowLeft": move(at.row, at.col - 1); return;
      case "ArrowRight": move(at.row, at.col + 1); return;
      case "Tab":
        if (event.shiftKey) {
          if (at.col > 0) move(at.row, at.col - 1);
          else if (at.row > 0) move(at.row - 1, lastCol);
        } else if (at.col < lastCol) move(at.row, at.col + 1);
        else if (at.row < lastRow) move(at.row + 1, 0);
        return;
      case "Enter":
        event.preventDefault();
        if ((event.metaKey || event.ctrlKey) && visible[at.col]?.id === "title") { onOpenRow(rows[at.row].note.id); return; }
        if (!selected) { select(0, 0); return; }
        openEditor(at.row, at.col);
        return;
      case " ":
        if (selected && visible[at.col]?.type === "checkbox") { event.preventDefault(); toggle(rows[at.row], visible[at.col]); }
        return;
      default:
        if (selected && event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey && EDITABLE_BY_TYPING.has(visible[at.col]?.type)) {
          event.preventDefault();
          openEditor(at.row, at.col, event.key);
        }
    }
  };

  const closeEditor = (reason?: CloseReason) => {
    setEditing(null);
    if (reason === "enter" && selected && selected.row < rows.length - 1) select(selected.row + 1, selected.col);
    grid.current?.focus({ preventScroll: true });
  };

  const clickCell = (row: number, col: number) => {
    grid.current?.focus({ preventScroll: true });
    if (selected?.row === row && selected.col === col && !editing) openEditor(row, col);
    else select(row, col);
  };

  const newRow = async () => {
    const id = await onNewRow();
    if (id) setMadeId(id);
  };

  // A system column not on the view yet can be shown from the `+` menu.
  const showable: Column[] = [
    ...columns.filter(column => !column.visible),
    ...SYSTEM_COLUMNS.filter(id => !columns.some(column => column.id === id)).map(id => ({ id, name: TYPE_LABELS[id], type: id, width: defaultWidth(id), visible: false })),
  ];
  const rowsWithValue = (column: Column) => rows.filter(row => { const value = row.values[column.id]; return value !== undefined && value !== null && !(Array.isArray(value) && !value.length); }).length;

  const cellStyle = (column: Column) => ({ width: widthOf(column), minWidth: widthOf(column) });

  return <div className="relative min-w-0">
    <div
      ref={scroller}
      onScroll={event => setScrollTop(event.currentTarget.scrollTop)}
      className={clsx("overflow-auto rounded-lg ring-1 ring-border", bounded ? "max-h-[420px]" : "max-h-[70vh]")}
    >
      <div
        ref={grid}
        role="grid"
        aria-label={`${database.properties.length ? "Database" : "Empty database"} table`}
        aria-rowcount={rows.length + 1}
        aria-colcount={visible.length}
        tabIndex={0}
        onKeyDown={onKeyDown}
        // Widths are per column and saved per view, so they are computed.
        style={{ width: totalWidth, minWidth: "100%" }}
        className="relative outline-none"
      >
        <div role="row" aria-rowindex={1} className="sticky top-0 z-20 flex h-9 border-b border-border bg-background">
          {visible.map((column, index) => <HeaderCell
            key={column.id}
            column={column}
            width={widthOf(column)}
            pinned={index === 0}
            open={menu === column.id}
            onOpen={() => setMenu(menu === column.id ? null : column.id)}
            onClose={() => setMenu(null)}
            onWidth={width => setWidths(current => { const next = { ...current }; if (width === null) delete next[column.id]; else next[column.id] = width; return next; })}
            onResized={width => { setWidths(current => ({ ...current, [column.id]: width })); actions.resize(column, width); }}
            menuBody={close => <ColumnMenu column={column} rowsWithValue={rowsWithValue(column)} actions={actions} onClose={close} />}
          />)}
          <AddColumn open={adding} onToggle={() => setAdding(current => !current)} onClose={() => setAdding(false)}>
            <AddProperty hidden={showable} onAdd={(name, type) => { setAdding(false); actions.addProperty(name, type); }} onShow={column => { setAdding(false); actions.show(column); }} />
          </AddColumn>
        </div>
        {start > 0 && <div aria-hidden="true" style={{ height: start * ROW_HEIGHT }} />}
        {rows.slice(start, end).map((row, offset) => {
          const index = start + offset;
          return <div key={row.note.id} role="row" aria-rowindex={index + 2} className="group flex h-9 border-b border-border last:border-b-0">
            {visible.map((column, col) => {
              const isSelected = selected?.row === index && selected.col === col;
              const isEditing = isSelected && !!editing;
              return <div
                key={column.id}
                role="gridcell"
                aria-selected={isSelected}
                aria-colindex={col + 1}
                // A click inside the open editor (its floating panel too, which
                // React bubbles here through the portal) is the editor's own.
                onClick={() => { if (!isEditing) clickCell(index, col); }}
                style={cellStyle(column)}
                className={clsx(
                  "relative shrink-0 border-r border-border",
                  col === 0 && "sticky left-0 z-10 bg-background",
                  isSelected && !isEditing && "ring-2 ring-inset ring-ring",
                )}
              >
                {column.id === "title" && !isEditing
                  ? <TitleCell title={row.note.title ?? ""} onOpen={() => onOpenRow(row.note.id)} />
                  : <PropertyCell
                    column={column}
                    value={cellValue(column, row.note, row.values)}
                    host={host}
                    mode="cell"
                    editing={isEditing}
                    seed={isEditing ? editing?.seed : undefined}
                    onEditingChange={(open, reason) => { if (!open) closeEditor(reason); }}
                    onCommit={value => onCommit(row, column, value)}
                  />}
              </div>;
            })}
          </div>;
        })}
        {end < rows.length && <div aria-hidden="true" style={{ height: (rows.length - end) * ROW_HEIGHT }} />}
        {!rows.length && <p className="m-0 px-3 py-6 text-[13px] text-muted-foreground">No rows yet</p>}
      </div>
    </div>
    <button type="button" onClick={() => void newRow()} className="mt-1 inline-flex h-8 items-center gap-1.5 rounded-md px-2 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <Plus size={14} aria-hidden="true" />New
    </button>
  </div>;
}

function TitleCell({ title, onOpen }: { title: string; onOpen: () => void }) {
  return <span className="flex h-full min-w-0 items-center gap-1 pl-2 pr-1 text-[13px]">
    <span className={clsx("min-w-0 flex-1 truncate font-medium", title ? "text-foreground" : "text-faint")}>{title || "Untitled"}</span>
    <button
      type="button"
      aria-label={`Open ${title || "Untitled"}`}
      onClick={event => { event.stopPropagation(); onOpen(); }}
      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-[11.5px] text-muted-foreground opacity-0 ring-1 ring-border transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
    ><ArrowUpRight size={12} aria-hidden="true" />Open</button>
  </span>;
}

function HeaderCell({ column, width, pinned, open, onOpen, onClose, onWidth, onResized, menuBody }: {
  column: Column;
  width: number;
  pinned: boolean;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  /** A live width while dragging; null drops it. */
  onWidth: (width: number | null) => void;
  onResized: (width: number) => void;
  menuBody: (close: () => void) => ReactNode;
}) {
  const anchor = useRef<HTMLDivElement>(null);
  const drag = (event: React.PointerEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const move = (moveEvent: PointerEvent) => onWidth(nextWidth(width, moveEvent.clientX - startX));
    const stop = (upEvent: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
      // A click, or a drag of under 2 px, saves nothing.
      if (upEvent.type === "pointerup" && Math.abs(upEvent.clientX - startX) >= 2) onResized(nextWidth(width, upEvent.clientX - startX));
      else onWidth(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
  };
  return <div
    ref={anchor}
    role="columnheader"
    aria-label={column.name}
    style={{ width, minWidth: width }}
    className={clsx("relative shrink-0 border-r border-border", pinned && "sticky left-0 z-10 bg-background")}
  >
    <button
      type="button"
      aria-haspopup="menu"
      aria-expanded={open}
      onClick={onOpen}
      className={clsx("flex h-full w-full min-w-0 items-center gap-1.5 px-2 text-left text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground", open && "bg-accent text-foreground")}
    >
      <span className="shrink-0">{typeIcon(column.type)}</span>
      <span className="min-w-0 truncate">{column.name}</span>
    </button>
    <span
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${column.name}`}
      onPointerDown={drag}
      className="absolute -right-1 top-0 z-10 h-full w-2 cursor-col-resize after:absolute after:inset-y-1 after:left-[3px] after:w-0.5 after:rounded-full after:bg-ring after:opacity-0 hover:after:opacity-100"
    />
    {open && <Floating anchor={anchor} label={`${column.name} column`} onClose={onClose} width="w-64">{menuBody(onClose)}</Floating>}
  </div>;
}

function AddColumn({ open, onToggle, onClose, children }: { open: boolean; onToggle: () => void; onClose: () => void; children: ReactNode }) {
  const anchor = useRef<HTMLDivElement>(null);
  return <div ref={anchor} style={{ width: ADD_WIDTH, minWidth: ADD_WIDTH }} className="relative shrink-0">
    <button type="button" aria-label="Add a property" aria-expanded={open} onClick={onToggle} className="grid h-full w-full place-items-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <Plus size={14} aria-hidden="true" />
    </button>
    {open && <Floating anchor={anchor} label="Add a property" onClose={onClose} width="w-72">{children}</Floating>}
  </div>;
}
