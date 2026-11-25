// The controls above a view: Filter, Sort and Group, editing an unsaved
// override that applies at once, plus "Save for everyone" and "Reset" while
// it differs from the saved view.

import { useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowUp, ArrowUpDown, Filter, Layers, Plus, X } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { Button } from "@/components/ui/button";
import { SelectField } from "@/components/ui/select";
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
      <Button variant="quiet" size="xs" onClick={() => { onReset(); setGeneration(current => current + 1); }}>Reset</Button>
      <Button size="xs" onClick={() => { onSave(); setGeneration(current => current + 1); }}>Save for everyone</Button>
    </span>}
  </div>;
}

function Control({ label, count, icon, width, children }: { label: string; count: number; icon: ReactNode; width: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement>(null);
  return <div ref={anchor} className="relative">
    <Button variant="quiet" size="xs" aria-expanded={open} aria-label={count ? `${label}, ${count}` : label} onClick={() => setOpen(current => !current)}>
      {icon}<span className={clsx(count > 0 && "text-foreground")}>{label}</span>{count > 0 && <span className="rounded bg-muted px-1 text-caption tabular-nums text-foreground">{count}</span>}
    </Button>
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
    {!sorts.length && <p className="m-0 px-1 py-1.5 text-meta text-muted-foreground">No sorts. Newest edits first.</p>}
    <ol className="m-0 flex list-none flex-col gap-1 p-0">
      {sorts.map((sort, index) => <li key={`${sort.property}-${index}`} role="group" aria-label={`Sort ${index + 1}`} className="flex items-center gap-1">
        <SelectField size="sm" label={`Sort ${index + 1} property`} className="min-w-0 flex-1" value={sort.property} onChange={property => onChange(sorts.map((item, at) => at === index ? { ...item, property } : item))} options={columns.map(column => ({ value: column.id, label: column.name }))} />
        <SelectField size="sm" label={`Sort ${index + 1} direction`} className="w-28 min-w-0" value={sort.direction} onChange={direction => onChange(sorts.map((item, at) => at === index ? { ...item, direction } : item))} options={[{ value: "asc", label: "Ascending" }, { value: "desc", label: "Descending" }]} />
        <IconButton label={`Move sort ${index + 1} up`} disabled={index === 0} onClick={() => move(index, -1)}><ArrowUp size={12} aria-hidden="true" /></IconButton>
        <IconButton label={`Move sort ${index + 1} down`} disabled={index === sorts.length - 1} onClick={() => move(index, 1)}><ArrowDown size={12} aria-hidden="true" /></IconButton>
        <IconButton label={`Remove sort ${index + 1}`} onClick={() => onChange(sorts.filter((_, at) => at !== index))}><X size={12} aria-hidden="true" /></IconButton>
      </li>)}
    </ol>
    {sorts.length < MAX_SORTS && unused && <Button variant="ghost" size="xs" className="w-fit" onClick={() => onChange([...sorts, { property: unused.id, direction: "asc" }])}><Plus aria-hidden="true" />Add a sort</Button>}
  </div>;
}

function GroupEditor({ database, groupBy, required, onChange }: { database: NoteDatabase; groupBy: NoteGroupBy | null; required: boolean; onChange: (groupBy: NoteGroupBy | null) => void }) {
  const columns = groupableColumns(database);
  const chosen = columns.find(column => column.id === groupBy?.property);
  return <div aria-label="Grouping" className="flex flex-col gap-1.5 p-1">
    <SelectField size="sm" label="Group by" value={groupBy?.property ?? ""} onChange={property => onChange(property ? { property, dateBucket: columns.find(column => column.id === property)?.type === "date" ? "month" : null } : null)} options={[...(!required || !groupBy ? [{ value: "", label: required ? "Choose a property" : "None" }] : []), ...columns.map(column => ({ value: column.id, label: column.name }))]} />
    {chosen?.type === "date" && <SelectField size="sm" label="Date bucket" value={groupBy?.dateBucket ?? "month"} onChange={dateBucket => onChange({ property: chosen.id, dateBucket })} options={[{ value: "day", label: "By day" }, { value: "week", label: "By week" }, { value: "month", label: "By month" }]} />}
  </div>;
}

function IconButton({ label, disabled, onClick, children }: { label: string; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <Button variant="quiet" size="icon-xs" aria-label={label} tooltip={label} disabled={disabled} onClick={onClick}>{children}</Button>;
}
