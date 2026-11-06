// A direct editor of #888's filter grammar: rows of property, op and value, a
// group one level deep, and/or per group. It keeps its own tree so a half-typed
// number stays on screen, and hands out only complete conditions.

import { useState } from "react";
import { Plus, X } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import type { NoteDatabase, Workspace } from "../../../types";
import { STATUS_GROUPS } from "./formatValue";
import { addCondition, addGroup, canAdd, choicesOf, completeOnly, defaultCondition, familyOf, filterColumns, isGroup, needsValue, OP_LABELS, opsFor, readFilter, removeAt, retarget, setCondition, setJoin, WITHIN, writeFilter, type Condition, type FieldColumn, type Group, type Path } from "./viewModel";

export interface FilterBuilderProps {
  database: NoteDatabase;
  /** The filter as #888's JSON. Read once: remount (a new `key`) to load another. */
  value: unknown;
  onChange: (filter: unknown) => void;
  workspaces?: Workspace[];
}

const control = "h-7 min-w-0 rounded-md bg-muted px-1.5 text-[12.5px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";
const LIST_OPS = new Set(["isAnyOf", "isNoneOf", "containsAny"]);

export function FilterBuilder({ database, value, onChange, workspaces = [] }: FilterBuilderProps) {
  const columns = filterColumns(database);
  const [tree, setTree] = useState<Group>(() => readFilter(value));
  const apply = (next: Group) => {
    setTree(next);
    onChange(writeFilter(completeOnly(next, columns)));
  };
  const first = columns[0];
  const add = (path: Path) => apply(addCondition(tree, path, defaultCondition(first)));

  const rows = (group: Group, path: Path) => group.items.map((item, index) => {
    const at = [...path, index];
    if (isGroup(item)) {
      return <li key={`g${index}`} className="rounded-lg bg-muted/50 p-1.5 ring-1 ring-border">
        <JoinSwitch label="Match in this group" join={item.join} onChange={join => apply(setJoin(tree, at, join))} />
        <ol className="m-0 mt-1 flex list-none flex-col gap-1 p-0">{rows(item, at)}</ol>
        <button type="button" disabled={!canAdd(tree)} onClick={() => add(at)} className="mt-1 inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[12px] text-muted-foreground transition-colors enabled:hover:bg-accent enabled:hover:text-foreground disabled:opacity-40"><Plus size={12} aria-hidden="true" />Add to group</button>
      </li>;
    }
    return <li key={`c${index}`}><ConditionRow condition={item} columns={columns} workspaces={workspaces} label={`Filter ${at.map(part => part + 1).join(".")}`} onChange={condition => apply(setCondition(tree, at, condition))} onRemove={() => apply(removeAt(tree, at))} /></li>;
  });

  return <div aria-label="Filter builder" className="flex w-[34rem] max-w-[calc(100vw-2rem)] flex-col gap-1.5 p-1">
    {tree.items.length > 1 && <JoinSwitch label="Match" join={tree.join} onChange={join => apply(setJoin(tree, [], join))} />}
    {tree.items.length
      ? <ol className="m-0 flex list-none flex-col gap-1 p-0">{rows(tree, [])}</ol>
      : <p className="m-0 px-1 py-1.5 text-[12.5px] text-muted-foreground">No filters. Every row shows.</p>}
    <div className="flex gap-1">
      <button type="button" disabled={!canAdd(tree)} onClick={() => add([])} className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[12.5px] text-foreground transition-colors enabled:hover:bg-accent disabled:opacity-40"><Plus size={13} aria-hidden="true" />Add a filter</button>
      <button type="button" disabled={!canAdd(tree)} onClick={() => apply(addGroup(tree, defaultCondition(first)))} className="inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[12.5px] text-muted-foreground transition-colors enabled:hover:bg-accent enabled:hover:text-foreground disabled:opacity-40"><Plus size={13} aria-hidden="true" />Add a filter group</button>
    </div>
  </div>;
}

function JoinSwitch({ label, join, onChange }: { label: string; join: "and" | "or"; onChange: (join: "and" | "or") => void }) {
  return <div role="radiogroup" aria-label={label} className="inline-flex w-fit rounded-md bg-muted p-0.5">
    {(["and", "or"] as const).map(option => <button key={option} type="button" role="radio" aria-checked={join === option} onClick={() => onChange(option)} className={clsx("h-6 rounded px-2 text-[12px] transition-colors", join === option ? "bg-background font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground")}>{option === "and" ? "All" : "Any"}</button>)}
  </div>;
}

function ConditionRow({ condition, columns, workspaces, label, onChange, onRemove }: { condition: Condition; columns: FieldColumn[]; workspaces: Workspace[]; label: string; onChange: (condition: Condition) => void; onRemove: () => void }) {
  const column = columns.find(candidate => candidate.id === condition.property) ?? columns[0];
  return <div role="group" aria-label={label} className="flex items-center gap-1">
    <select aria-label={`${label} property`} value={column.id} onChange={event => onChange(defaultCondition(columns.find(candidate => candidate.id === event.target.value)!))} className={clsx(control, "w-32 shrink-0")}>
      {columns.map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.name}</option>)}
    </select>
    <select aria-label={`${label} op`} value={condition.op} onChange={event => onChange(retarget(condition, column, event.target.value))} className={clsx(control, "w-36 shrink-0")}>
      {opsFor(column).map(op => <option key={op} value={op}>{OP_LABELS[op] ?? op}</option>)}
    </select>
    <div className="min-w-0 flex-1">{needsValue(condition.op) && <ValueInput condition={condition} column={column} workspaces={workspaces} label={`${label} value`} onChange={value => onChange({ ...condition, value })} />}</div>
    <button type="button" aria-label={`Remove ${label.toLowerCase()}`} onClick={onRemove} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><X size={13} aria-hidden="true" /></button>
  </div>;
}

function ValueInput({ condition, column, workspaces, label, onChange }: { condition: Condition; column: FieldColumn; workspaces: Workspace[]; label: string; onChange: (value: unknown) => void }) {
  const family = familyOf(column.type);
  const value = condition.value;
  if (condition.op === "within") {
    return <select aria-label={label} value={String(value ?? "pastWeek")} onChange={event => onChange(event.target.value)} className={clsx(control, "w-full")}>{WITHIN.map(([id, words]) => <option key={id} value={id}>{words}</option>)}</select>;
  }
  if (condition.op === "inGroup") {
    return <select aria-label={label} value={String(value ?? "todo")} onChange={event => onChange(event.target.value)} className={clsx(control, "w-full")}>{STATUS_GROUPS.map(([id, words]) => <option key={id} value={id}>{words}</option>)}</select>;
  }
  const choices = choicesOf(column, workspaces);
  if (LIST_OPS.has(condition.op) && choices.length) {
    const chosen = Array.isArray(value) ? value.map(String) : [];
    return <div role="group" aria-label={label} className="flex flex-wrap gap-1">
      {choices.map(([id, name]) => <label key={id} className={clsx("inline-flex h-6 cursor-pointer items-center gap-1 rounded-md px-1.5 text-[12px]", chosen.includes(id) ? "bg-selection text-selection-foreground" : "bg-muted text-muted-foreground")}>
        <input type="checkbox" className="sr-only" checked={chosen.includes(id)} onChange={() => onChange(chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id])} />{name}
      </label>)}
    </div>;
  }
  if (family === "checkbox") {
    return <select aria-label={label} value={value === true ? "true" : "false"} onChange={event => onChange(event.target.value === "true")} className={clsx(control, "w-full")}><option value="true">Checked</option><option value="false">Not checked</option></select>;
  }
  if ((family === "choice" || family === "list") && choices.length) {
    return <select aria-label={label} value={String(value ?? "")} onChange={event => onChange(event.target.value)} className={clsx(control, "w-full")}>{choices.map(([id, name]) => <option key={id} value={id}>{name}</option>)}</select>;
  }
  if (family === "number") {
    return <NumberInput label={label} value={value} onChange={onChange} />;
  }
  if (family === "date") {
    return <input aria-label={label} type="date" value={typeof value === "string" ? value.slice(0, 10) : ""} onChange={event => onChange(event.target.value)} className={clsx(control, "w-full")} />;
  }
  return <input aria-label={label} value={typeof value === "string" ? value : Array.isArray(value) ? value.join(", ") : ""} onChange={event => onChange(LIST_OPS.has(condition.op) ? event.target.value.split(",").map(item => item.trim()).filter(Boolean) : event.target.value)} className={clsx(control, "w-full")} />;
}

/** A number typed as text: anything that is not a finite number stays on
 *  screen and is left out of the filter until it is. */
function NumberInput({ label, value, onChange }: { label: string; value: unknown; onChange: (value: unknown) => void }) {
  const [text, setText] = useState(typeof value === "number" && Number.isFinite(value) ? String(value) : "");
  const parsed = text.trim() === "" ? Number.NaN : Number(text);
  return <input
    aria-label={label}
    inputMode="decimal"
    aria-invalid={!Number.isFinite(parsed) || undefined}
    value={text}
    onChange={event => { setText(event.target.value); const next = event.target.value.trim() === "" ? Number.NaN : Number(event.target.value); onChange(next); }}
    className={clsx(control, "w-full tabular-nums", !Number.isFinite(parsed) && "ring-1 ring-destructive")}
  />;
}
