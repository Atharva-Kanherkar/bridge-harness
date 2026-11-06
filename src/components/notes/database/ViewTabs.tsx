// A database's views as tabs: switch, make a new one, and from each tab's
// menu rename, duplicate, change type, move or delete it. Dragging a tab onto
// another moves it there. The last view cannot be deleted.

import { useRef, useState, type ReactNode } from "react";
import { CalendarDays, Kanban, List, MoreHorizontal, Plus, Table2 } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { NoteDatabaseView } from "../../../types";
import { MenuDivider, MenuLabel } from "../NotesUi";
import { Floating } from "./Floating";

export const VIEW_TYPES: [string, string][] = [["table", "Table"], ["board", "Board"], ["calendar", "Calendar"], ["list", "List"]];
const ICONS: Record<string, typeof Table2> = { table: Table2, board: Kanban, calendar: CalendarDays, list: List };
export const viewIcon = (type: string, size = 13) => {
  const Icon = ICONS[type] ?? Table2;
  return <Icon size={size} strokeWidth={1.75} aria-hidden="true" />;
};

export interface ViewActions {
  create: (type: string, name: string) => void;
  rename: (view: NoteDatabaseView, name: string) => void;
  duplicate: (view: NoteDatabaseView) => void;
  changeType: (view: NoteDatabaseView, type: string) => void;
  remove: (view: NoteDatabaseView) => void;
  /** Put `view` at `index` in the shown order. */
  move: (view: NoteDatabaseView, index: number) => void;
}

export function ViewTabs({ views, activeId, onSelect, actions }: { views: NoteDatabaseView[]; activeId: string | null; onSelect: (id: string) => void; actions: ViewActions }) {
  const [dragging, setDragging] = useState<string | null>(null);
  return <div role="tablist" aria-label="Views" className="mb-2 flex flex-wrap items-center gap-0.5">
    {views.map((view, index) => <Tab
      key={view.id}
      view={view}
      index={index}
      count={views.length}
      active={view.id === activeId}
      onSelect={() => onSelect(view.id)}
      actions={actions}
      onDragStart={() => setDragging(view.id)}
      onDrop={() => {
        const from = views.find(candidate => candidate.id === dragging);
        setDragging(null);
        if (from && from.id !== view.id) actions.move(from, index);
      }}
    />)}
    <NewView onCreate={actions.create} />
  </div>;
}

function Tab({ view, index, count, active, onSelect, actions, onDragStart, onDrop }: { view: NoteDatabaseView; index: number; count: number; active: boolean; onSelect: () => void; actions: ViewActions; onDragStart: () => void; onDrop: () => void }) {
  const [menu, setMenu] = useState<"main" | "rename" | "type" | null>(null);
  const [name, setName] = useState(view.name);
  const anchor = useRef<HTMLDivElement>(null);
  const close = () => setMenu(null);
  return <div
    ref={anchor}
    draggable
    onDragStart={event => { event.dataTransfer?.setData("text/plain", view.id); onDragStart(); }}
    onDragOver={event => event.preventDefault()}
    onDrop={event => { event.preventDefault(); onDrop(); }}
    className={clsx("group relative inline-flex items-center rounded-md", active ? "bg-accent" : "hover:bg-accent/60")}
  >
    <button type="button" role="tab" aria-selected={active} onClick={onSelect} className={clsx("inline-flex h-7 items-center gap-1.5 pl-2 pr-1 text-[12.5px] transition-colors", active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}>
      {viewIcon(view.type)}{view.name}
    </button>
    <button type="button" aria-label={`${view.name} view menu`} aria-expanded={!!menu} onClick={() => { setName(view.name); setMenu(menu ? null : "main"); }} className={clsx("grid size-6 place-items-center rounded text-muted-foreground transition-opacity hover:text-foreground", active || menu ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100")}>
      <MoreHorizontal size={13} aria-hidden="true" />
    </button>
    {menu && <Floating anchor={anchor} label={`${view.name} view`} onClose={close} width="w-56">
      {menu === "rename"
        ? <form onSubmit={event => { event.preventDefault(); if (name.trim() && name.trim() !== view.name) actions.rename(view, name.trim()); close(); }} className="p-1">
          <input aria-label="View name" autoFocus value={name} onChange={event => setName(event.target.value)} className="h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring" />
        </form>
        : menu === "type"
          ? <div role="menu" aria-label="View type">
            <MenuLabel>Change type</MenuLabel>
            {VIEW_TYPES.map(([type, label]) => <MenuItem key={type} checked={view.type === type} icon={viewIcon(type)} onClick={() => { if (type !== view.type) actions.changeType(view, type); close(); }}>{label}</MenuItem>)}
          </div>
          : <div role="menu" aria-label={`${view.name} actions`}>
            <MenuItem onClick={() => setMenu("rename")}>Rename</MenuItem>
            <MenuItem onClick={() => { actions.duplicate(view); close(); }}>Duplicate</MenuItem>
            <MenuItem onClick={() => setMenu("type")}>Change type</MenuItem>
            <MenuItem disabled={index === 0} onClick={() => { actions.move(view, index - 1); close(); }}>Move left</MenuItem>
            <MenuItem disabled={index === count - 1} onClick={() => { actions.move(view, index + 1); close(); }}>Move right</MenuItem>
            <MenuDivider />
            <MenuItem destructive disabled={count <= 1} onClick={() => { actions.remove(view); close(); }}>{count <= 1 ? "Delete (the last view stays)" : "Delete"}</MenuItem>
          </div>}
    </Floating>}
  </div>;
}

function NewView({ onCreate }: { onCreate: (type: string, name: string) => void }) {
  const [open, setOpen] = useState(false);
  const [type, setType] = useState("table");
  const [name, setName] = useState("");
  const anchor = useRef<HTMLDivElement>(null);
  const label = VIEW_TYPES.find(([id]) => id === type)![1];
  return <div ref={anchor} className="relative">
    <button type="button" aria-expanded={open} onClick={() => { setOpen(current => !current); setName(""); }} className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Plus size={13} aria-hidden="true" />New view</button>
    {open && <Floating anchor={anchor} label="New view" onClose={() => setOpen(false)} width="w-64">
      <form onSubmit={event => { event.preventDefault(); onCreate(type, name.trim() || label); setOpen(false); }} className="flex flex-col gap-1.5 p-1">
        <div role="radiogroup" aria-label="View type" className="grid grid-cols-2 gap-1">
          {VIEW_TYPES.map(([id, words]) => <button key={id} type="button" role="radio" aria-checked={type === id} onClick={() => setType(id)} className={clsx("inline-flex h-8 items-center gap-1.5 rounded-lg px-2 text-[12.5px] transition-colors", type === id ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")}>{viewIcon(id)}{words}</button>)}
        </div>
        <input aria-label="New view name" value={name} placeholder={label} onChange={event => setName(event.target.value)} className="h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring" />
        <button type="submit" className="inline-flex h-8 items-center justify-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground hover:opacity-90">Create</button>
      </form>
    </Floating>}
  </div>;
}

function MenuItem({ checked, icon, destructive, disabled, onClick, children }: { checked?: boolean; icon?: ReactNode; destructive?: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" role={checked === undefined ? "menuitem" : "menuitemradio"} aria-checked={checked} disabled={disabled} onClick={onClick} className={clsx("flex min-h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[13px] transition-colors enabled:hover:bg-accent disabled:opacity-40", destructive ? "text-destructive" : "text-foreground", checked && "font-medium")}>
    {icon && <span className="text-muted-foreground">{icon}</span>}<span className="min-w-0 flex-1 truncate">{children}</span>
  </button>;
}
