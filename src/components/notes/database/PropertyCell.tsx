// One editor per property type, the same in the table, a row page's property
// panel and (later) a board card. A cell shows its value through
// `formatValue`; editing commits the stored shape, `null` to clear, and never
// an unchanged value. Escape always cancels.
//
// In a table cell (`mode: "cell"`) the table owns clicks and keys and opens
// the editor through `editing`; in a panel the cell is its own button.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowUpRight, Check, Plus } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { openExternalUrl } from "../../../externalLinks";
import type { NoteOption, NoteOptionInput, NoteProperty, Workspace } from "../../../types";
import { MenuLabel, Option, OptionList } from "../NotesUi";
import { Floating } from "./Floating";
import { STATUS_GROUPS, formatValue, type Column } from "./formatValue";
import { useResetOnChange } from "../../../useResetOnChange";

/** What a cell needs from the database around it. */
export interface CellHost {
  /** Add an option to the property's stored list; resolves to the property as stored, or null if refused. */
  addOption: (property: NoteProperty, option: NoteOptionInput) => Promise<NoteProperty | null>;
  /** Rows of a database matching `query`, for a relation. */
  searchRows: (databaseId: string, query: string) => Promise<{ id: string; title: string }[]>;
  rowTitle: (id: string) => string | null | undefined;
  workspaces: Workspace[];
  chatTitle?: (sessionId: string) => string | undefined;
  onOpenChat?: (sessionId: string) => void;
}

export type CloseReason = "enter" | "escape" | "blur" | "pick";

export interface PropertyCellProps {
  column: Pick<Column, "id" | "name" | "type" | "property">;
  value: unknown;
  host: CellHost;
  mode?: "cell" | "panel";
  /** Controlled editing (the table); a panel cell keeps its own. */
  editing?: boolean;
  /** Text typed to open the editor, which seeds it. */
  seed?: string;
  onEditingChange?: (editing: boolean, reason?: CloseReason) => void;
  onCommit: (value: unknown) => void;
}

const INLINE = new Set(["title", "text", "number", "url"]);
const READ_ONLY = new Set(["created", "updated"]);
const OPENABLE = /^(https?:|mailto:)/i;

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

export function PropertyCell({ column, value, host, mode = "panel", editing: controlled, seed, onEditingChange, onCommit }: PropertyCellProps) {
  const [own, setOwn] = useState(false);
  const editing = controlled ?? own;
  const anchor = useRef<HTMLDivElement>(null);
  const setEditing = (next: boolean, reason?: CloseReason) => {
    if (controlled === undefined) setOwn(next);
    onEditingChange?.(next, reason);
  };
  const commit = (next: unknown, reason: CloseReason) => {
    if (!same(next, value)) onCommit(next);
    setEditing(reason === "pick" && column.type === "multiSelect", reason);
  };
  const cancel = () => setEditing(false, "escape");
  const type = column.type;

  // A read-only column has no editor to open.
  // `setEditing` is the table's, so closing a read-only editor waits for the commit.
  // oxlint-disable-next-line react/set-state-in-effect -- the editing state belongs to the parent table
  useEffect(() => { if (editing && READ_ONLY.has(type)) setEditing(false, "escape"); }); // eslint-disable-line react-hooks/exhaustive-deps

  const text = formatValue(column, value, { rowTitle: host.rowTitle, workspaceTitle: id => host.workspaces.find(workspace => workspace.id === id)?.title });
  const display = <Display column={column} value={value} text={text} host={host} mode={mode} />;

  if (editing && INLINE.has(type)) {
    return <InlineInput type={type} name={column.name} initial={seed ?? (type === "number" ? (typeof value === "number" ? String(value) : "") : typeof value === "string" ? value : "")} mode={mode} onCommit={(next, reason) => commit(next, reason)} onCancel={cancel} />;
  }

  const panelButton = (children: ReactNode) => <RowButton
    aria-label={column.name}
    aria-haspopup={type === "checkbox" ? undefined : "dialog"}
    aria-expanded={type === "checkbox" ? undefined : editing}
    aria-pressed={type === "checkbox" ? value === true : undefined}
    disabled={READ_ONLY.has(type)}
    onClick={() => type === "checkbox" ? onCommit(!(value === true)) : setEditing(!editing)}
    onKeyDown={event => { if (type === "checkbox" && event.key === " ") { event.preventDefault(); onCommit(!(value === true)); } }}
    active={editing}
    className="-ml-1.5"
  ><span className={clsx("flex min-w-0 flex-1 items-center", text || type === "checkbox" ? "text-foreground" : "text-faint")}>{children}</span></RowButton>;

  const openable = type === "url" && typeof value === "string" && OPENABLE.test(value.trim());
  return <div ref={anchor} className={clsx("relative min-w-0", mode === "cell" ? "h-full" : "flex items-center gap-1")}>
    {mode === "panel" ? panelButton(text || type === "checkbox" ? display : "Empty") : display}
    {/* Beside the panel's own button, never inside it. */}
    {mode === "panel" && openable && <OpenLink url={value} />}
    {editing && !INLINE.has(type) && type !== "checkbox" && !READ_ONLY.has(type) && <Floating anchor={anchor} label={`${column.name} editor`} onClose={cancel}>
      {type === "select" || type === "status" || type === "multiSelect"
        ? <OptionPicker property={column.property!} value={value} seed={seed} host={host} onPick={next => commit(next, "pick")} onCancel={cancel} />
        : type === "date"
          ? <DateEditor name={column.name} value={value} onCommit={next => commit(next, "enter")} onCancel={cancel} />
          : type === "relation"
            ? <RelationPicker property={column.property!} value={value} host={host} onPick={next => { if (!same(next, value)) onCommit(next); }} onCancel={cancel} />
            : type === "project"
              ? <OptionList label="Project" className="max-h-56 overflow-y-auto">
                <Option selected={!value} onSelect={() => commit(null, "pick")}>No project</Option>
                {host.workspaces.map(workspace => <Option key={workspace.id} selected={value === workspace.id} onSelect={() => commit(workspace.id, "pick")}>{workspace.title}</Option>)}
              </OptionList>
              : type === "chats"
                ? <ChatLinks value={value} host={host} />
                : null}
    </Floating>}
  </div>;
}

/** The value as a cell or panel reads it. */
function Display({ column, value, text, host, mode }: { column: PropertyCellProps["column"]; value: unknown; text: string; host: CellHost; mode: "cell" | "panel" }) {
  const box = mode === "cell" ? "flex h-full min-w-0 items-center px-2 text-ui" : "flex min-w-0 flex-wrap items-center gap-1 py-1 [overflow-wrap:anywhere]";
  switch (column.type) {
    case "checkbox":
      return <span className={box}><span aria-label={value === true ? "Checked" : "Not checked"} className={clsx("grid size-4 shrink-0 place-items-center rounded border", value === true ? "border-foreground bg-foreground text-background" : "border-border")}>{value === true && <Check size={12} strokeWidth={2.5} aria-hidden="true" />}</span></span>;
    case "select": case "status": case "multiSelect": {
      const ids = Array.isArray(value) ? value : value ? [value] : [];
      const options = ids.map(id => column.property?.options?.find(option => option.id === id)).filter((option): option is NoteOption => !!option);
      return <span className={clsx(box, "gap-1 overflow-hidden")}>{options.map(option => <Chip key={option.id} option={option} status={column.type === "status"} />)}</span>;
    }
    case "url":
      return <span className={clsx(box, "gap-1")}>
        <span className={clsx("min-w-0 underline decoration-border underline-offset-4", mode === "cell" && "truncate")}>{text}</span>
        {mode === "cell" && typeof value === "string" && OPENABLE.test(value.trim()) && <OpenLink url={value} />}
      </span>;
    case "chats":
      return mode === "panel" && Array.isArray(value) && value.length
        ? <span className={clsx(box, "gap-2")}>{value.map(id => <span key={String(id)} className="truncate">{host.chatTitle?.(String(id)) ?? "A chat"}</span>)}</span>
        : <span className={clsx(box, "truncate")}>{text}</span>;
    default:
      return mode === "cell"
        ? <span className={clsx(box, column.type === "number" && "justify-end tabular-nums")}><span className="min-w-0 truncate">{text}</span></span>
        : <span className={box}>{text}</span>;
  }
}

/** Opens a cell's link the way Bridge opens any outside link. */
function OpenLink({ url }: { url: string }) {
  return <Button
    variant="quiet"
    size="icon-2xs"
    aria-label={`Open link ${url}`}
    tooltip="Open link"
    onClick={event => { event.stopPropagation(); void openExternalUrl(url.trim()); }}
    onMouseDown={event => event.stopPropagation()}
  ><ArrowUpRight aria-hidden="true" /></Button>;
}

function Chip({ option, status }: { option: NoteOption; status: boolean }) {
  return <span className="inline-flex max-w-full shrink-0 items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-meta leading-4 text-foreground">
    {status && <span aria-hidden="true" className={clsx("size-1.5 shrink-0 rounded-full", option.group === "done" ? "bg-foreground" : option.group === "inProgress" ? "bg-muted-foreground" : "border border-muted-foreground")} />}
    <span className="truncate">{option.name}</span>
  </span>;
}

function InlineInput({ type, name, initial, mode, onCommit, onCancel }: { type: string; name: string; initial: string; mode: "cell" | "panel"; onCommit: (value: unknown, reason: CloseReason) => void; onCancel: () => void }) {
  const [text, setText] = useState(initial);
  const done = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const box = input.current;
    if (!box) return;
    box.focus();
    box.setSelectionRange(box.value.length, box.value.length);
  }, []);
  const invalid = type === "number" && text.trim() !== "" && !Number.isFinite(Number(text.trim()));
  const parsed = (): unknown => {
    if (type === "number") return text.trim() === "" ? null : Number(text.trim());
    if (type === "title") return text.replace(/\s+$/, "");
    const trimmed = text.replace(/\s+$/, "");
    return trimmed === "" ? null : trimmed;
  };
  const finish = (reason: CloseReason) => {
    if (done.current) return;
    if (invalid) {
      if (reason === "blur") { done.current = true; onCancel(); }
      return;
    }
    done.current = true;
    onCommit(parsed(), reason);
  };
  // oxlint-disable-next-line bridge/no-raw-controls -- the in-place cell editor fills its grid cell; a boxed field would shift the row
  return <input
    ref={input}
    aria-label={name}
    aria-invalid={invalid || undefined}
    inputMode={type === "number" ? "decimal" : undefined}
    type={type === "url" ? "url" : "text"}
    value={text}
    onChange={event => setText(event.target.value)}
    onKeyDown={event => {
      event.stopPropagation();
      if (event.nativeEvent.isComposing) return;
      if (event.key === "Enter") { event.preventDefault(); finish("enter"); }
      if (event.key === "Escape") { event.preventDefault(); done.current = true; onCancel(); }
    }}
    onBlur={() => finish("blur")}
    className={clsx(
      "min-w-0 bg-background text-ui text-foreground outline-none",
      mode === "cell" ? "h-full w-full px-2 ring-2 ring-inset ring-ring" : "h-8 w-full rounded-md px-1.5 ring-1 ring-ring",
      invalid && "ring-destructive",
      type === "number" && mode === "cell" && "text-right tabular-nums",
    )}
  />;
}

function SearchBox({ label, value, onChange, onEnter, onCancel }: { label: string; value: string; onChange: (value: string) => void; onEnter: () => void; onCancel: () => void }) {
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => { box.current?.focus(); }, []);
  return <Input
    ref={box}
    size="sm"
    className="mb-1"
    aria-label={label}
    value={value}
    placeholder="Search or create…"
    onChange={event => onChange(event.target.value)}
    onKeyDown={event => {
      if (event.nativeEvent.isComposing) return;
      if (event.key === "Enter") { event.preventDefault(); onEnter(); }
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onCancel(); }
    }}
  />;
}

const fold = (text: string) => text.trim().toLowerCase();

function OptionPicker({ property, value, seed, host, onPick, onCancel }: { property: NoteProperty; value: unknown; seed?: string; host: CellHost; onPick: (value: unknown) => void; onCancel: () => void }) {
  const multi = property.type === "multiSelect";
  const [query, setQuery] = useState(seed ?? "");
  const [options, setOptions] = useState<NoteOption[]>(property.options ?? []);
  const [creating, setCreating] = useState(false);
  useResetOnChange([property.options], () => setOptions(property.options ?? []));
  const chosen: string[] = multi ? (Array.isArray(value) ? value.map(String) : []) : value ? [String(value)] : [];
  const matches = options.filter(option => !query.trim() || fold(option.name).includes(fold(query)));
  const exact = options.some(option => fold(option.name) === fold(query));
  const pick = (id: string) => {
    if (multi) onPick(chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id]);
    else onPick(chosen.includes(id) ? null : id);
  };
  const create = async () => {
    const name = query.trim();
    if (!name || exact || creating) return;
    setCreating(true);
    // Added to the list as stored, never this picker's copy of it.
    const stored = await host.addOption(property, { name, ...(property.type === "status" ? { group: "todo" } : {}) });
    setCreating(false);
    const made = stored?.options?.find(option => fold(option.name) === fold(name));
    if (!stored || !made) return;
    setOptions(stored.options ?? []);
    setQuery("");
    pick(made.id);
  };
  const row = (option: NoteOption) => <Option key={option.id} selected={chosen.includes(option.id)} onSelect={() => pick(option.id)}>{option.name}</Option>;
  return <div>
    <SearchBox label={`Search ${property.name} options`} value={query} onChange={setQuery} onCancel={onCancel} onEnter={() => { if (matches[0]) pick(matches[0].id); else void create(); }} />
    <OptionList label={`${property.name} options`} className="max-h-64 overflow-y-auto">
      {property.type === "status"
        ? STATUS_GROUPS.map(([group, label]) => {
          const inGroup = matches.filter(option => (option.group ?? "todo") === group);
          return inGroup.length ? <div key={group} role="group" aria-label={label}><MenuLabel>{label}</MenuLabel>{inGroup.map(row)}</div> : null;
        })
        : matches.map(row)}
      {query.trim() && !exact && <RowButton shape="card" onClick={() => void create()} disabled={creating}>
        <Plus size={14} aria-hidden="true" className="shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">Create <span className="font-medium">{query.trim()}</span></span>
      </RowButton>}
      {!matches.length && !query.trim() && <p className="m-0 px-2.5 py-2 text-meta text-muted-foreground">No options yet. Type one to create it.</p>}
    </OptionList>
  </div>;
}

/** `YYYY-MM-DDTHH:mm` for a `datetime-local` input, in the device's zone. */
function localInput(raw: string): string {
  if (!raw.includes("T")) return `${raw}T09:00`;
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** A `datetime-local` value as RFC 3339 with the device's offset. */
export function withOffset(local: string): string | null {
  const date = new Date(local);
  if (!local || Number.isNaN(date.getTime())) return null;
  const pad = (n: number) => String(Math.trunc(Math.abs(n))).padStart(2, "0");
  const offset = -date.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  return `${local.length === 16 ? `${local}:00` : local}${sign}${pad(offset / 60)}:${pad(offset % 60)}`;
}

function DateEditor({ name, value, onCommit, onCancel }: { name: string; value: unknown; onCommit: (value: unknown) => void; onCancel: () => void }) {
  const stored = (value && typeof value === "object" ? value : {}) as { start?: string; end?: string };
  const [withTime, setWithTime] = useState(!!stored.start?.includes("T"));
  const [hasEnd, setHasEnd] = useState(!!stored.end);
  const [start, setStart] = useState(stored.start ? (withTime ? localInput(stored.start) : stored.start.slice(0, 10)) : "");
  const [end, setEnd] = useState(stored.end ? (withTime ? localInput(stored.end) : stored.end.slice(0, 10)) : "");
  const toggleTime = () => {
    setWithTime(current => {
      setStart(value => value ? (current ? value.slice(0, 10) : `${value.slice(0, 10)}T09:00`) : value);
      setEnd(value => value ? (current ? value.slice(0, 10) : `${value.slice(0, 10)}T09:00`) : value);
      return !current;
    });
  };
  const read = (raw: string) => raw ? (withTime ? withOffset(raw) : raw) : null;
  const startValue = read(start);
  const endValue = hasEnd ? read(end) : null;
  // Both days or both instants, since the time toggle switches both inputs.
  const backwards = !!startValue && !!endValue && (withTime ? Date.parse(endValue) < Date.parse(startValue) : endValue < startValue);
  const ready = !!startValue && !backwards;
  const save = () => { if (ready) onCommit(endValue ? { start: startValue, end: endValue } : { start: startValue }); };
  const keys = (event: React.KeyboardEvent) => {
    if (event.key === "Enter") { event.preventDefault(); save(); }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onCancel(); }
  };
  return <div className="flex flex-col gap-1.5 p-1" onKeyDown={keys}>
    <Input size="sm" aria-label={`${name} start`} type={withTime ? "datetime-local" : "date"} value={start} onChange={event => setStart(event.target.value)} autoFocus />
    {hasEnd && <Input size="sm" aria-label={`${name} end`} type={withTime ? "datetime-local" : "date"} value={end} onChange={event => setEnd(event.target.value)} />}
    {backwards && <p role="alert" className="m-0 px-1 text-meta text-destructive">The end is before the start.</p>}
    <label className="flex items-center gap-2 px-1 text-meta text-foreground"><Checkbox checked={hasEnd} onCheckedChange={() => setHasEnd(current => !current)} />End date</label>
    <label className="flex items-center gap-2 px-1 text-meta text-foreground"><Checkbox checked={withTime} onCheckedChange={toggleTime} />Include time</label>
    <div className="mt-1 flex justify-between gap-1">
      <Button variant="quiet" size="sm" onClick={() => onCommit(null)}>Clear</Button>
      <Button size="sm" onClick={save} disabled={!ready}>Done</Button>
    </div>
  </div>;
}

function RelationPicker({ property, value, host, onPick, onCancel }: { property: NoteProperty; value: unknown; host: CellHost; onPick: (value: unknown) => void; onCancel: () => void }) {
  const target = property.relation?.databaseId ?? "";
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ id: string; title: string }[]>([]);
  const [chosen, setChosen] = useState<string[]>(Array.isArray(value) ? value.map(String) : []);
  // Every title a search has shown, so a chosen page keeps its name after it
  // drops out of the results.
  const [titles, setTitles] = useState<ReadonlyMap<string, string>>(() => new Map());
  const ticket = useRef(0);
  useEffect(() => {
    if (!target) return;
    const mine = ++ticket.current;
    void host.searchRows(target, query).then(rows => {
      if (mine !== ticket.current) return;
      setTitles(current => {
        const next = new Map(current);
        for (const row of rows) next.set(row.id, row.title);
        return next;
      });
      setResults(rows);
    }, () => undefined);
  }, [target, query, host]);
  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id];
    setChosen(next);
    onPick(next);
  };
  const name = (id: string) => titles.get(id) ?? host.rowTitle(id) ?? "Untitled";
  const others = useMemo(() => results.filter(row => !chosen.includes(row.id)), [results, chosen]);
  return <div>
    <SearchBox label={`Search ${property.name} pages`} value={query} onChange={setQuery} onCancel={onCancel} onEnter={() => { if (others[0]) toggle(others[0].id); }} />
    <OptionList label={`${property.name} pages`} className="max-h-64 overflow-y-auto">
      {chosen.map(id => <Option key={id} selected onSelect={() => toggle(id)}>{name(id) || "Untitled"}</Option>)}
      {others.map(row => <Option key={row.id} selected={false} onSelect={() => toggle(row.id)}>{row.title || "Untitled"}</Option>)}
      {!target && <p className="m-0 px-2.5 py-2 text-meta text-muted-foreground">This relation has no database to point at.</p>}
      {target && !chosen.length && !others.length && <p className="m-0 px-2.5 py-2 text-meta text-muted-foreground">No pages match.</p>}
    </OptionList>
  </div>;
}

function ChatLinks({ value, host }: { value: unknown; host: CellHost }) {
  const ids = Array.isArray(value) ? value.map(String) : [];
  if (!ids.length) return <p className="m-0 px-2.5 py-2 text-meta text-muted-foreground">No linked chats.</p>;
  return <div className="flex flex-col">
    {ids.map(id => <RowButton key={id} shape="card" onClick={() => host.onOpenChat?.(id)}><span className="truncate underline decoration-border underline-offset-4">{host.chatTitle?.(id) ?? "A chat"}</span></RowButton>)}
  </div>;
}
