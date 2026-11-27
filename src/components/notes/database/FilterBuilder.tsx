// A direct editor of #888's filter grammar: rows of property, op and value, a
// group one level deep, and/or per group. It keeps its own tree so a half-typed
// number stays on screen, and hands out only complete conditions.

import { useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SelectField } from "@/components/ui/select";
import { Segmented } from "@/components/ui/segmented";
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

const LIST_OPS = new Set(["isAnyOf", "isNoneOf", "containsAny"]);

export function FilterBuilder({ database, value, onChange, workspaces = [] }: FilterBuilderProps) {
  const columns = filterColumns(database);
  const [tree, setTree] = useState<Group>(() => readFilter(value));
  // Each row keeps one key through edits, so its half-typed text stays with
  // its own condition when a row above it goes. An edit makes a new object;
  // it takes the key of the object it replaced at the same place.
  const keys = useRef(new WeakMap<object, string>());
  const counter = useRef(0);
  const keyOf = (item: object) => {
    let key = keys.current.get(item);
    if (!key) { key = `r${++counter.current}`; keys.current.set(item, key); }
    return key;
  };
  const carry = (before: Group, after: Group) => {
    after.items.forEach((item, index) => {
      const old = before.items[index];
      if (!keys.current.has(item) && old && isGroup(old) === isGroup(item) && !after.items.includes(old)) keys.current.set(item, keyOf(old));
      if (isGroup(item) && old && isGroup(old)) carry(old, item);
    });
  };
  const apply = (next: Group) => {
    carry(tree, next);
    setTree(next);
    onChange(writeFilter(completeOnly(next, columns)));
  };
  const first = columns[0];
  const add = (path: Path) => apply(addCondition(tree, path, defaultCondition(first, workspaces)));

  const rows = (group: Group, path: Path) => group.items.map((item, index) => {
    const at = [...path, index];
    if (isGroup(item)) {
      return <li key={keyOf(item)} className="rounded-lg bg-muted/50 p-1.5 ring-1 ring-border">
        <JoinSwitch label="Match in this group" join={item.join} onChange={join => apply(setJoin(tree, at, join))} />
        <ol className="m-0 mt-1 flex list-none flex-col gap-1 p-0">{rows(item, at)}</ol>
        <Button variant="quiet" size="xs" disabled={!canAdd(tree)} onClick={() => add(at)} className="mt-1"><Plus aria-hidden="true" />Add to group</Button>
      </li>;
    }
    return <li key={keyOf(item)}><ConditionRow condition={item} columns={columns} workspaces={workspaces} label={`Filter ${at.map(part => part + 1).join(".")}`} onChange={condition => apply(setCondition(tree, at, condition))} onRemove={() => apply(removeAt(tree, at))} /></li>;
  });

  return <div aria-label="Filter builder" className="flex w-[34rem] max-w-[calc(100vw-2rem)] flex-col gap-1.5 p-1">
    {tree.items.length > 1 && <JoinSwitch label="Match" join={tree.join} onChange={join => apply(setJoin(tree, [], join))} />}
    {tree.items.length
      // `keyOf` hands each condition object a stable key the first time it is
      // drawn; the WeakMap is a cache keyed by identity, so render fills it.
      // oxlint-disable-next-line react/refs -- an identity-keyed cache of row keys
      ? <ol className="m-0 flex list-none flex-col gap-1 p-0">{rows(tree, [])}</ol>
      : <p className="m-0 px-1 py-1.5 text-meta text-muted-foreground">No filters. Every row shows.</p>}
    <div className="flex gap-1">
      <Button variant="ghost" size="xs" disabled={!canAdd(tree)} onClick={() => add([])}><Plus aria-hidden="true" />Add a filter</Button>
      <Button variant="quiet" size="xs" disabled={!canAdd(tree)} onClick={() => apply(addGroup(tree, defaultCondition(first, workspaces)))}><Plus aria-hidden="true" />Add a filter group</Button>
    </div>
  </div>;
}

function JoinSwitch({ label, join, onChange }: { label: string; join: "and" | "or"; onChange: (join: "and" | "or") => void }) {
  return <Segmented label={label} className="w-fit" value={join} options={[{ value: "and" as const, label: "All" }, { value: "or" as const, label: "Any" }]} onChange={onChange} />;
}

function ConditionRow({ condition, columns, workspaces, label, onChange, onRemove }: { condition: Condition; columns: FieldColumn[]; workspaces: Workspace[]; label: string; onChange: (condition: Condition) => void; onRemove: () => void }) {
  const column = columns.find(candidate => candidate.id === condition.property) ?? columns[0];
  return <div role="group" aria-label={label} className="flex items-center gap-1">
    <SelectField size="sm" label={`${label} property`} className="w-32 min-w-0 shrink-0" value={column.id} onChange={next => onChange(defaultCondition(columns.find(candidate => candidate.id === next)!, workspaces))} options={columns.map(candidate => ({ value: candidate.id, label: candidate.name }))} />
    <SelectField size="sm" label={`${label} op`} className="w-36 min-w-0 shrink-0" value={condition.op} onChange={next => onChange(retarget(condition, column, next, workspaces))} options={opsFor(column).map(op => ({ value: op, label: OP_LABELS[op] ?? op }))} />
    <div className="min-w-0 flex-1">{needsValue(condition.op) && <ValueInput condition={condition} column={column} workspaces={workspaces} label={`${label} value`} onChange={value => onChange({ ...condition, value })} />}</div>
    <Button variant="quiet" size="icon-xs" aria-label={`Remove ${label.toLowerCase()}`} tooltip="Remove" onClick={onRemove}><X aria-hidden="true" /></Button>
  </div>;
}

function ValueInput({ condition, column, workspaces, label, onChange }: { condition: Condition; column: FieldColumn; workspaces: Workspace[]; label: string; onChange: (value: unknown) => void }) {
  const family = familyOf(column.type);
  const value = condition.value;
  if (condition.op === "within") {
    return <SelectField size="sm" label={label} className="min-w-0" value={String(value ?? "pastWeek")} onChange={onChange} options={WITHIN.map(([id, words]) => ({ value: id, label: words }))} />;
  }
  if (condition.op === "inGroup") {
    return <SelectField size="sm" label={label} className="min-w-0" value={String(value ?? "todo")} onChange={onChange} options={STATUS_GROUPS.map(([id, words]) => ({ value: id, label: words }))} />;
  }
  const choices = choicesOf(column, workspaces);
  if (LIST_OPS.has(condition.op) && choices.length) {
    const chosen = Array.isArray(value) ? value.map(String) : [];
    return <div role="group" aria-label={label} className="flex flex-wrap gap-1">
      {choices.map(([id, name]) => <Button key={id} variant="chip" size="2xs" aria-pressed={chosen.includes(id)} onClick={() => onChange(chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id])}>{name}</Button>)}
    </div>;
  }
  if (family === "checkbox") {
    return <SelectField size="sm" label={label} className="min-w-0" value={value === true ? "true" : "false"} onChange={next => onChange(next === "true")} options={[{ value: "true", label: "Checked" }, { value: "false", label: "Not checked" }]} />;
  }
  if ((family === "choice" || family === "list") && choices.length) {
    // A value that names no choice shows as unchosen, never as the first one.
    const chosen = choices.some(([id]) => id === value) ? String(value) : "";
    return <SelectField size="sm" label={label} className="min-w-0" value={chosen} placeholder="Choose…" onChange={onChange} options={choices.map(([id, name]) => ({ value: id, label: name }))} />;
  }
  if (family === "number") {
    return <NumberInput label={label} value={value} onChange={onChange} />;
  }
  if (family === "date") {
    return <Input size="sm" aria-label={label} type="date" value={typeof value === "string" ? value.slice(0, 10) : ""} onChange={event => onChange(event.target.value)} />;
  }
  return <Input size="sm" aria-label={label} value={typeof value === "string" ? value : Array.isArray(value) ? value.join(", ") : ""} onChange={event => onChange(LIST_OPS.has(condition.op) ? event.target.value.split(",").map(item => item.trim()).filter(Boolean) : event.target.value)} />;
}

/** A number typed as text: anything that is not a finite number stays on
 *  screen and is left out of the filter until it is. */
function NumberInput({ label, value, onChange }: { label: string; value: unknown; onChange: (value: unknown) => void }) {
  const [text, setText] = useState(typeof value === "number" && Number.isFinite(value) ? String(value) : "");
  const parsed = text.trim() === "" ? Number.NaN : Number(text);
  // The row's condition can change under the field (a row above removed, the
  // property switched): show the number it now applies. Half-typed text that
  // is not yet a number stays.
  useEffect(() => {
    if (typeof value === "number" && Number.isFinite(value) && value !== parsed) setText(String(value));
  }, [value]); // eslint-disable-line react-hooks/exhaustive-deps
  return <Input
    size="sm"
    aria-label={label}
    inputMode="decimal"
    aria-invalid={!Number.isFinite(parsed) || undefined}
    value={text}
    onChange={event => { setText(event.target.value); const next = event.target.value.trim() === "" ? Number.NaN : Number(event.target.value); onChange(next); }}
  />;
}
