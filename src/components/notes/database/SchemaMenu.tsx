// A column's header menu and the "Add a property" form. Each action is one
// call: a schema change (rename, retype, options, delete) or a view save
// (sort, hide). The menu says what a retype keeps before it is chosen, and
// how many rows lose a value before a delete.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowDownAZ, ArrowLeft, ArrowUp, ArrowUpAZ, EyeOff, ListChecks, Trash2, Type, X } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { NoteOptionInput, NoteProperty } from "../../../types";
import { MenuDivider, MenuLabel } from "../NotesUi";
import { OPTION_TYPES, PROPERTY_TYPES, STATUS_GROUPS, TYPE_LABELS, retypeNote, type Column } from "./formatValue";

export interface ColumnActions {
  rename: (property: NoteProperty, name: string) => void;
  retype: (property: NoteProperty, type: string) => void;
  /** `seen`: the option ids the editor opened with, so options added since are kept. */
  setOptions: (property: NoteProperty, options: NoteOptionInput[], seen: string[]) => void;
  sort: (column: Column, direction: "asc" | "desc") => void;
  hide: (column: Column) => void;
  remove: (property: NoteProperty) => void;
}

function Item({ icon, destructive, onClick, children }: { icon?: ReactNode; destructive?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" role="menuitem" onClick={onClick} className={clsx("flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none", destructive ? "text-destructive" : "text-foreground")}>
    {icon && <span className="grid w-4 shrink-0 place-items-center text-muted-foreground">{icon}</span>}
    <span className="min-w-0 flex-1 truncate">{children}</span>
  </button>;
}

const field = "h-8 w-full min-w-0 rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring";

/** The menu a column header opens. `rowsWithValue` counts loaded rows that
 *  hold a value, for the delete confirmation. */
export function ColumnMenu({ column, rowsWithValue, actions, onClose }: { column: Column; rowsWithValue: number; actions: ColumnActions; onClose: () => void }) {
  const [page, setPage] = useState<"main" | "type" | "options" | "delete">("main");
  const property = column.property;
  const [name, setName] = useState(column.name);
  const done = (run: () => void) => { run(); onClose(); };
  const renameNow = () => {
    if (property && name.trim() && name.trim() !== property.name) actions.rename(property, name.trim());
  };

  if (page === "type" && property) {
    return <div role="menu" aria-label={`${column.name} type`}>
      <Back onClick={() => setPage("main")}>Type</Back>
      {PROPERTY_TYPES.filter(type => type !== "relation" || property.type === "relation").map(type => <button
        key={type}
        type="button"
        role="menuitemradio"
        aria-checked={property.type === type}
        onClick={() => done(() => { if (type !== property.type) actions.retype(property, type); })}
        className="flex w-full flex-col items-start rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-accent"
      >
        <span className={clsx("text-[13px] text-foreground", property.type === type && "font-medium")}>{TYPE_LABELS[type]}</span>
        <span className="text-[11.5px] text-muted-foreground">{retypeNote(property.type, type)}</span>
      </button>)}
    </div>;
  }
  if (page === "options" && property) {
    return <OptionsEditor property={property} onBack={() => setPage("main")} onSave={(options, seen) => done(() => actions.setOptions(property, options, seen))} />;
  }
  if (page === "delete" && property) {
    return <div role="alertdialog" aria-label={`Delete ${property.name}`} className="p-1.5">
      <p className="m-0 text-[13px] leading-snug text-foreground">Delete {property.name}? {rowsWithValue ? `${rowsWithValue} row${rowsWithValue === 1 ? " has" : "s have"} a value.` : "No row has a value."}</p>
      <p className="m-0 mt-1 text-[12px] text-muted-foreground">Its values stay in page history.</p>
      <div className="mt-3 flex justify-end gap-1">
        <button type="button" onClick={() => setPage("main")} className="inline-flex h-8 items-center rounded-full px-3 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Cancel</button>
        <button type="button" onClick={() => done(() => actions.remove(property))} className="inline-flex h-8 items-center rounded-full bg-destructive px-3.5 text-[13px] font-medium text-destructive-foreground hover:opacity-90">Delete</button>
      </div>
    </div>;
  }
  return <div role="menu" aria-label={`${column.name} menu`}>
    {property && <input
      aria-label="Property name"
      value={name}
      onChange={event => setName(event.target.value)}
      onBlur={renameNow}
      onKeyDown={event => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") { event.preventDefault(); done(renameNow); }
      }}
      className={clsx(field, "mb-1")}
    />}
    {property && <Item icon={<Type size={14} aria-hidden="true" />} onClick={() => setPage("type")}>Type: {TYPE_LABELS[property.type] ?? property.type}</Item>}
    {property && OPTION_TYPES.includes(property.type) && <Item icon={<ListChecks size={14} aria-hidden="true" />} onClick={() => setPage("options")}>Edit options</Item>}
    {property && <MenuDivider />}
    {column.type !== "chats" && <>
      <Item icon={<ArrowUpAZ size={14} aria-hidden="true" />} onClick={() => done(() => actions.sort(column, "asc"))}>Sort ascending</Item>
      <Item icon={<ArrowDownAZ size={14} aria-hidden="true" />} onClick={() => done(() => actions.sort(column, "desc"))}>Sort descending</Item>
    </>}
    {column.id !== "title" && <Item icon={<EyeOff size={14} aria-hidden="true" />} onClick={() => done(() => actions.hide(column))}>Hide</Item>}
    {property && <>
      <MenuDivider />
      <Item destructive icon={<Trash2 size={14} aria-hidden="true" />} onClick={() => setPage("delete")}>Delete property</Item>
    </>}
  </div>;
}

function Back({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return <button type="button" onClick={onClick} className="mb-1 flex h-7 w-full items-center gap-1.5 rounded-lg px-1.5 text-left text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
    <ArrowLeft size={13} aria-hidden="true" />{children}
  </button>;
}

interface Draft { key: string; id?: string; name: string; group?: string | null; color?: string | null }

/** Every option of a select, multi-select or status, edited as one list:
 *  saved whole, in its order. A new option carries no id; a missing one is
 *  deleted, and its values cleared, by the core. */
export function OptionsEditor({ property, onSave, onBack }: { property: NoteProperty; onSave: (options: NoteOptionInput[], seen: string[]) => void; onBack: () => void }) {
  const status = property.type === "status";
  const counter = useRef(0);
  const [seen] = useState(() => (property.options ?? []).map(option => option.id));
  const [drafts, setDrafts] = useState<Draft[]>(() => (property.options ?? []).map(option => ({ key: option.id, id: option.id, name: option.name, group: option.group, color: option.color })));
  const [adding, setAdding] = useState("");
  const patch = (key: string, change: Partial<Draft>) => setDrafts(current => current.map(draft => draft.key === key ? { ...draft, ...change } : draft));
  const move = (index: number, by: number) => setDrafts(current => {
    const next = [...current];
    const [item] = next.splice(index, 1);
    next.splice(index + by, 0, item);
    return next;
  });
  const add = () => {
    const name = adding.trim();
    if (!name) return;
    setDrafts(current => [...current, { key: `new-${++counter.current}`, name, ...(status ? { group: "todo" } : {}) }]);
    setAdding("");
  };
  const names = drafts.map(draft => draft.name.trim().toLowerCase());
  const problem = names.some(name => !name) ? "Every option needs a name." : new Set(names).size !== names.length ? "Two options have the same name." : null;
  const save = () => {
    if (problem) return;
    onSave(drafts.map(draft => ({ ...(draft.id ? { id: draft.id } : {}), name: draft.name.trim(), ...(draft.color ? { color: draft.color } : {}), ...(status ? { group: draft.group ?? "todo" } : {}) })), seen);
  };
  return <div aria-label={`${property.name} options`}>
    <Back onClick={onBack}>Options</Back>
    <ol className="m-0 flex max-h-72 list-none flex-col gap-1 overflow-y-auto p-0">
      {drafts.map((draft, index) => <li key={draft.key} className="flex items-center gap-1">
        <input aria-label={`Option ${index + 1} name`} value={draft.name} onChange={event => patch(draft.key, { name: event.target.value })} className={field} />
        {status && <select aria-label={`Option ${index + 1} group`} value={draft.group ?? "todo"} onChange={event => patch(draft.key, { group: event.target.value })} className="h-8 shrink-0 rounded-lg bg-muted px-1.5 text-[12px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring">
          {STATUS_GROUPS.map(([group, label]) => <option key={group} value={group}>{label}</option>)}
        </select>}
        <IconButton label={`Move ${draft.name || "option"} up`} disabled={index === 0} onClick={() => move(index, -1)}><ArrowUp size={13} aria-hidden="true" /></IconButton>
        <IconButton label={`Move ${draft.name || "option"} down`} disabled={index === drafts.length - 1} onClick={() => move(index, 1)}><ArrowDown size={13} aria-hidden="true" /></IconButton>
        <IconButton label={`Delete ${draft.name || "option"}`} onClick={() => setDrafts(current => current.filter(other => other.key !== draft.key))}><X size={13} aria-hidden="true" /></IconButton>
      </li>)}
    </ol>
    <input
      aria-label="New option"
      value={adding}
      placeholder="Add an option"
      onChange={event => setAdding(event.target.value)}
      onKeyDown={event => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); add(); } }}
      className={clsx(field, "mt-1.5")}
    />
    {problem && <p role="alert" className="m-0 mt-1.5 px-1 text-[12px] text-destructive">{problem}</p>}
    <div className="mt-2 flex justify-end">
      <button type="button" onClick={save} disabled={!!problem} className="inline-flex h-8 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40">Save options</button>
    </div>
  </div>;
}

function IconButton({ label, disabled, onClick, children }: { label: string; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" aria-label={label} title={label} disabled={disabled} onClick={onClick} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors enabled:hover:bg-accent enabled:hover:text-foreground disabled:opacity-30">{children}</button>;
}

/** The `+` at the end of the header: a new property's name and type, and the
 *  hidden columns to show again. */
export function AddProperty({ hidden, onAdd, onShow }: { hidden: Column[]; onAdd: (name: string, type: string) => void; onShow?: (column: Column) => void }) {
  const [name, setName] = useState("");
  const [type, setType] = useState<string>("text");
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => { box.current?.focus(); }, []);
  const submit = () => { if (name.trim()) onAdd(name.trim(), type); };
  return <div aria-label="Add a property">
    <MenuLabel>Add a property</MenuLabel>
    <input
      ref={box}
      aria-label="New property name"
      value={name}
      placeholder="Name"
      onChange={event => setName(event.target.value)}
      onKeyDown={event => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); submit(); } }}
      className={field}
    />
    <div role="radiogroup" aria-label="New property type" className="mt-1.5 grid grid-cols-3 gap-1">
      {PROPERTY_TYPES.filter(candidate => candidate !== "relation").map(candidate => <button
        key={candidate}
        type="button"
        role="radio"
        aria-checked={type === candidate}
        onClick={() => setType(candidate)}
        className={clsx("h-7 truncate rounded-md px-1.5 text-[12px] transition-colors", type === candidate ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")}
      >{TYPE_LABELS[candidate]}</button>)}
    </div>
    <div className="mt-2 flex justify-end">
      <button type="button" onClick={submit} disabled={!name.trim()} className="inline-flex h-8 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40">Add</button>
    </div>
    {hidden.length > 0 && onShow && <>
      <MenuDivider />
      <MenuLabel>Hidden</MenuLabel>
      {hidden.map(column => <div key={column.id} className="flex min-h-8 items-center gap-2 px-2.5 text-[13px]">
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{column.name}</span>
        <button type="button" aria-label={`Show ${column.name}`} onClick={() => onShow(column)} className="rounded-md px-1.5 py-0.5 text-[12px] text-foreground transition-colors hover:bg-accent">Show</button>
      </div>)}
    </>}
  </div>;
}
