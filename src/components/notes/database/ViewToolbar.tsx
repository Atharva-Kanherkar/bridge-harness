// The controls above a view: Filter, Sort and Group, editing an unsaved
// override that applies at once, plus "Save for everyone" and "Reset" while
// it differs from the saved view.

import { useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowUp, ArrowUpDown, Filter, Layers, Plus, X } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { NoteDatabase, NoteDatabaseView, NoteGroupBy, NoteSort, Workspace } from "../../../types";
import { Floating } from "./Floating";
import { FilterBuilder } from "./FilterBuilder";
import type { ViewQuery } from "./useViewOverride";
import { countConditions, groupableColumns, readFilter, sortableColumns } from "./viewModel";

export const MAX_SORTS = 5;

export interface ViewToolbarProps {
  database: NoteDatabase;
  view: NoteDatabaseView;
  query: ViewQuery;
  dirty: boolean;
  onChange: (patch: Partial<ViewQuery>) => void;
  onSave: () => void;
  onReset: () => void;
  workspaces?: Workspace[];
}

const control = "h-7 min-w-0 rounded-md bg-muted px-1.5 text-[12.5px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function ViewToolbar({ database, view, query, dirty, onChange, onSave, onReset, workspaces }: ViewToolbarProps) {
  // A reset reloads the filter builder from the saved view.
  const [generation, setGeneration] = useState(0);
  const filters = countConditions(readFilter(query.filter));
  return <div role="toolbar" aria-label="View options" className="mb-2 flex flex-wrap items-center gap-1">
    <Control label="Filter" count={filters} icon={<Filter size={13} aria-hidden="true" />} width="w-auto">
      <FilterBuilder key={`${view.id}-${generation}`} database={database} value={query.filter} onChange={filter => onChange({ filter })} workspaces={workspaces} />
    </Control>
    <Control label="Sort" count={query.sorts.length} icon={<ArrowUpDown size={13} aria-hidden="true" />} width="w-96">
      <SortEditor database={database} sorts={query.sorts} onChange={sorts => onChange({ sorts })} />
    </Control>
    {view.type !== "calendar" && <Control label="Group" count={query.groupBy ? 1 : 0} icon={<Layers size={13} aria-hidden="true" />} width="w-80">
      <GroupEditor database={database} groupBy={query.groupBy} required={view.type === "board"} onChange={groupBy => onChange({ groupBy })} />
    </Control>}
    {dirty && <span className="ml-auto flex items-center gap-1">
      <button type="button" onClick={() => { onReset(); setGeneration(current => current + 1); }} className="inline-flex h-7 items-center rounded-full px-2.5 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Reset</button>
      <button type="button" onClick={() => { onSave(); setGeneration(current => current + 1); }} className="inline-flex h-7 items-center rounded-full bg-primary px-3 text-[12.5px] font-medium text-primary-foreground transition-opacity hover:opacity-90">Save for everyone</button>
    </span>}
  </div>;
}

function Control({ label, count, icon, width, children }: { label: string; count: number; icon: ReactNode; width: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement>(null);
  return <div ref={anchor} className="relative">
    <button type="button" aria-expanded={open} aria-label={count ? `${label}, ${count}` : label} onClick={() => setOpen(current => !current)} className={clsx("inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12.5px] transition-colors hover:bg-accent", count ? "text-foreground" : "text-muted-foreground", open && "bg-accent")}>
      {icon}{label}{count > 0 && <span className="rounded bg-muted px-1 text-[11px] tabular-nums">{count}</span>}
    </button>
    {open && <Floating anchor={anchor} label={`${label} options`} onClose={() => setOpen(false)} width={width}>{children}</Floating>}
  </div>;
}

function SortEditor({ database, sorts, onChange }: { database: NoteDatabase; sorts: NoteSort[]; onChange: (sorts: NoteSort[]) => void }) {
  const columns = sortableColumns(database);
  const move = (index: number, by: number) => {
    const next = [...sorts];
    const [item] = next.splice(index, 1);
    next.splice(index + by, 0, item);
    onChange(next);
  };
  const unused = columns.find(column => !sorts.some(sort => sort.property === column.id));
  return <div aria-label="Sorts" className="flex flex-col gap-1 p-1">
    {!sorts.length && <p className="m-0 px-1 py-1.5 text-[12.5px] text-muted-foreground">No sorts. Newest edits first.</p>}
    <ol className="m-0 flex list-none flex-col gap-1 p-0">
      {sorts.map((sort, index) => <li key={`${sort.property}-${index}`} role="group" aria-label={`Sort ${index + 1}`} className="flex items-center gap-1">
        <select aria-label={`Sort ${index + 1} property`} value={sort.property} onChange={event => onChange(sorts.map((item, at) => at === index ? { ...item, property: event.target.value } : item))} className={clsx(control, "flex-1")}>
          {columns.map(column => <option key={column.id} value={column.id}>{column.name}</option>)}
        </select>
        <select aria-label={`Sort ${index + 1} direction`} value={sort.direction} onChange={event => onChange(sorts.map((item, at) => at === index ? { ...item, direction: event.target.value } : item))} className={clsx(control, "w-28")}>
          <option value="asc">Ascending</option>
          <option value="desc">Descending</option>
        </select>
        <IconButton label={`Move sort ${index + 1} up`} disabled={index === 0} onClick={() => move(index, -1)}><ArrowUp size={12} aria-hidden="true" /></IconButton>
        <IconButton label={`Move sort ${index + 1} down`} disabled={index === sorts.length - 1} onClick={() => move(index, 1)}><ArrowDown size={12} aria-hidden="true" /></IconButton>
        <IconButton label={`Remove sort ${index + 1}`} onClick={() => onChange(sorts.filter((_, at) => at !== index))}><X size={12} aria-hidden="true" /></IconButton>
      </li>)}
    </ol>
    {sorts.length < MAX_SORTS && unused && <button type="button" onClick={() => onChange([...sorts, { property: unused.id, direction: "asc" }])} className="inline-flex h-7 w-fit items-center gap-1 rounded-md px-1.5 text-[12.5px] text-foreground transition-colors hover:bg-accent"><Plus size={13} aria-hidden="true" />Add a sort</button>}
  </div>;
}

function GroupEditor({ database, groupBy, required, onChange }: { database: NoteDatabase; groupBy: NoteGroupBy | null; required: boolean; onChange: (groupBy: NoteGroupBy | null) => void }) {
  const columns = groupableColumns(database);
  const chosen = columns.find(column => column.id === groupBy?.property);
  return <div aria-label="Grouping" className="flex flex-col gap-1.5 p-1">
    <select aria-label="Group by" value={groupBy?.property ?? ""} onChange={event => onChange(event.target.value ? { property: event.target.value, dateBucket: columns.find(column => column.id === event.target.value)?.type === "date" ? "month" : null } : null)} className={clsx(control, "w-full")}>
      {(!required || !groupBy) && <option value="">{required ? "Choose a property" : "None"}</option>}
      {columns.map(column => <option key={column.id} value={column.id}>{column.name}</option>)}
    </select>
    {chosen?.type === "date" && <select aria-label="Date bucket" value={groupBy?.dateBucket ?? "month"} onChange={event => onChange({ property: chosen.id, dateBucket: event.target.value })} className={clsx(control, "w-full")}>
      <option value="day">By day</option>
      <option value="week">By week</option>
      <option value="month">By month</option>
    </select>}
  </div>;
}

function IconButton({ label, disabled, onClick, children }: { label: string; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" aria-label={label} title={label} disabled={disabled} onClick={onClick} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors enabled:hover:bg-accent enabled:hover:text-foreground disabled:opacity-30">{children}</button>;
}
