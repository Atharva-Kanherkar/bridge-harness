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
import { openExternalUrl } from "../../../externalLinks";
import type { NoteOption, NoteOptionInput, NoteProperty, Workspace } from "../../../types";
import { MenuLabel, Option, OptionList } from "../NotesUi";
import { Floating } from "./Floating";
import { STATUS_GROUPS, formatValue, type Column } from "./formatValue";

/** What a cell needs from the database around it. */
export interface CellHost {
  /** Replace a property's options; resolves to the property as stored, or null if refused. */
  setOptions: (property: NoteProperty, options: NoteOptionInput[]) => Promise<NoteProperty | null>;
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
  useEffect(() => { if (editing && READ_ONLY.has(type)) setEditing(false, "escape"); }); // eslint-disable-line react-hooks/exhaustive-deps

  const text = formatValue(column, value, { rowTitle: host.rowTitle, workspaceTitle: id => host.workspaces.find(workspace => workspace.id === id)?.title });
  const display = <Display column={column} value={value} text={text} host={host} mode={mode} />;

  if (editing && INLINE.has(type)) {
    return <InlineInput type={type} name={column.name} initial={seed ?? (type === "number" ? (typeof value === "number" ? String(value) : "") : typeof value === "string" ? value : "")} mode={mode} onCommit={(next, reason) => commit(next, reason)} onCancel={cancel} />;
  }

  const panelButton = (children: ReactNode) => <button
    type="button"
    aria-label={column.name}
    aria-haspopup={type === "checkbox" ? undefined : "dialog"}
    aria-expanded={type === "checkbox" ? undefined : editing}
    aria-pressed={type === "checkbox" ? value === true : undefined}
    disabled={READ_ONLY.has(type)}
    onClick={() => type === "checkbox" ? onCommit(!(value === true)) : setEditing(!editing)}
    onKeyDown={event => { if (type === "checkbox" && event.key === " ") { event.preventDefault(); onCommit(!(value === true)); } }}
    className={clsx("-ml-1.5 flex min-h-8 w-full min-w-0 items-center rounded-md px-1.5 text-left text-[13px] transition-colors enabled:hover:bg-accent", editing && "bg-accent", text || type === "checkbox" ? "text-foreground" : "text-faint")}
  >{children}</button>;

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
  const box = mode === "cell" ? "flex h-full min-w-0 items-center px-2 text-[13px]" : "flex min-w-0 flex-wrap items-center gap-1 py-1 [overflow-wrap:anywhere]";
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
  return <button
    type="button"
    aria-label={`Open link ${url}`}
    title="Open link"
    onClick={event => { event.stopPropagation(); void openExternalUrl(url.trim()); }}
    onMouseDown={event => event.stopPropagation()}
    className="grid size-5 shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
  ><ArrowUpRight size={13} aria-hidden="true" /></button>;
}

function Chip({ option, status }: { option: NoteOption; status: boolean }) {
  return <span className="inline-flex max-w-full shrink-0 items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[12px] leading-4 text-foreground">
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
      "min-w-0 bg-background text-[13px] text-foreground outline-none",
      mode === "cell" ? "h-full w-full px-2 ring-2 ring-inset ring-ring" : "h-8 w-full rounded-md px-1.5 ring-1 ring-ring",
      invalid && "ring-destructive",
      type === "number" && mode === "cell" && "text-right tabular-nums",
    )}
  />;
}

function SearchBox({ label, value, onChange, onEnter, onCancel }: { label: string; value: string; onChange: (value: string) => void; onEnter: () => void; onCancel: () => void }) {
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => { box.current?.focus(); }, []);
  return <input
    ref={box}
    aria-label={label}
    value={value}
    placeholder="Search or create…"
    onChange={event => onChange(event.target.value)}
    onKeyDown={event => {
      if (event.nativeEvent.isComposing) return;
      if (event.key === "Enter") { event.preventDefault(); onEnter(); }
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onCancel(); }
    }}
    className="mb-1 h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none placeholder:text-faint focus-visible:ring-2 focus-visible:ring-ring"
  />;
}

const fold = (text: string) => text.trim().toLowerCase();

function OptionPicker({ property, value, seed, host, onPick, onCancel }: { property: NoteProperty; value: unknown; seed?: string; host: CellHost; onPick: (value: unknown) => void; onCancel: () => void }) {
  const multi = property.type === "multiSelect";
  const [query, setQuery] = useState(seed ?? "");
  const [options, setOptions] = useState<NoteOption[]>(property.options ?? []);
  const [creating, setCreating] = useState(false);
  useEffect(() => setOptions(property.options ?? []), [property.options]);
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
    const inputs: NoteOptionInput[] = [...options.map(option => ({ id: option.id, name: option.name, color: option.color ?? null, group: option.group ?? null })), { name, ...(property.type === "status" ? { group: "todo" } : {}) }];
    const stored = await host.setOptions(property, inputs);
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
      {query.trim() && !exact && <button type="button" onClick={() => void create()} disabled={creating} className="flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] text-foreground transition-colors hover:bg-accent disabled:opacity-60">
        <Plus size={14} aria-hidden="true" className="shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">Create <span className="font-medium">{query.trim()}</span></span>
      </button>}
      {!matches.length && !query.trim() && <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">No options yet. Type one to create it.</p>}
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
  const field = "h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";
  return <div className="flex flex-col gap-1.5 p-1" onKeyDown={keys}>
    <input aria-label={`${name} start`} type={withTime ? "datetime-local" : "date"} value={start} onChange={event => setStart(event.target.value)} autoFocus className={field} />
    {hasEnd && <input aria-label={`${name} end`} type={withTime ? "datetime-local" : "date"} value={end} onChange={event => setEnd(event.target.value)} className={field} />}
    {backwards && <p role="alert" className="m-0 px-1 text-[12px] text-destructive">The end is before the start.</p>}
    <label className="flex items-center gap-2 px-1 text-[12.5px] text-foreground"><input type="checkbox" checked={hasEnd} onChange={() => setHasEnd(current => !current)} className="accent-foreground" />End date</label>
    <label className="flex items-center gap-2 px-1 text-[12.5px] text-foreground"><input type="checkbox" checked={withTime} onChange={toggleTime} className="accent-foreground" />Include time</label>
    <div className="mt-1 flex justify-between gap-1">
      <button type="button" onClick={() => onCommit(null)} className="inline-flex h-8 items-center rounded-full px-3 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Clear</button>
      <button type="button" onClick={save} disabled={!ready} className="inline-flex h-8 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40">Done</button>
    </div>
  </div>;
}

function RelationPicker({ property, value, host, onPick, onCancel }: { property: NoteProperty; value: unknown; host: CellHost; onPick: (value: unknown) => void; onCancel: () => void }) {
  const target = property.relation?.databaseId ?? "";
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<{ id: string; title: string }[]>([]);
  const [chosen, setChosen] = useState<string[]>(Array.isArray(value) ? value.map(String) : []);
  const titles = useRef(new Map<string, string>());
  const ticket = useRef(0);
  useEffect(() => {
    if (!target) return;
    const mine = ++ticket.current;
    void host.searchRows(target, query).then(rows => {
      if (mine !== ticket.current) return;
      for (const row of rows) titles.current.set(row.id, row.title);
      setResults(rows);
    }, () => undefined);
  }, [target, query, host]);
  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter(item => item !== id) : [...chosen, id];
    setChosen(next);
    onPick(next);
  };
  const name = (id: string) => titles.current.get(id) ?? host.rowTitle(id) ?? "Untitled";
  const others = useMemo(() => results.filter(row => !chosen.includes(row.id)), [results, chosen]);
  return <div>
    <SearchBox label={`Search ${property.name} pages`} value={query} onChange={setQuery} onCancel={onCancel} onEnter={() => { if (others[0]) toggle(others[0].id); }} />
    <OptionList label={`${property.name} pages`} className="max-h-64 overflow-y-auto">
      {chosen.map(id => <Option key={id} selected onSelect={() => toggle(id)}>{name(id) || "Untitled"}</Option>)}
      {others.map(row => <Option key={row.id} selected={false} onSelect={() => toggle(row.id)}>{row.title || "Untitled"}</Option>)}
      {!target && <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">This relation has no database to point at.</p>}
      {target && !chosen.length && !others.length && <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">No pages match.</p>}
    </OptionList>
  </div>;
}

function ChatLinks({ value, host }: { value: unknown; host: CellHost }) {
  const ids = Array.isArray(value) ? value.map(String) : [];
  if (!ids.length) return <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">No linked chats.</p>;
  return <div className="flex flex-col">
    {ids.map(id => <button key={id} type="button" onClick={() => host.onOpenChat?.(id)} className="flex min-h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-foreground underline decoration-border underline-offset-4 transition-colors hover:bg-accent">{host.chatTitle?.(id) ?? "A chat"}</button>)}
  </div>;
}
