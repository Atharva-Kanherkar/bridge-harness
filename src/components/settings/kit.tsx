// The Settings design system, in one file.
//
// The rule every page here obeys: **a page is a column of groups, a group is a
// card of rows, a row is one label and one control.** Nothing else appears on a
// settings page. Before this kit existed, eight sections had four content
// widths, ten type sizes, five card styles, and five different save patterns,
// because each one drew its own layout. A page that can only compose these
// primitives cannot drift that way again.
//
// Desktop controls and group rhythm are shared with the main destinations.
// Settings remain rows of labels and controls with quiet explanatory text.
//
// Shared controls use the same Lucide symbols as the rest of Bridge.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown as CaretDown, ChevronRight as CaretRight, Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { Select as SelectRoot, SelectGroup, SelectGroupLabel, SelectItem, SelectPopup, SelectTrigger } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Switch as SwitchControl } from "@/components/ui/switch";
import { SETTINGS_COLUMN } from "./layout";

export { Segmented } from "@/components/ui/segmented";

/* ── page ─────────────────────────────────────────────────────────────────── */

export type Crumb = { label: string; onClick?: () => void };

export { SETTINGS_COLUMN, SETTINGS_COLUMN_WIDE } from "./layout";

/**
 * One settings page, the same anatomy everywhere: an optional breadcrumb, a
 * title, one line of description, an optional action, then labeled groups of
 * rows and at most one `SettingsFold`.
 */
export function SettingsPage({ title, description, breadcrumb, action, children }: {
  title: string;
  description?: string;
  breadcrumb?: Crumb[];
  action?: ReactNode;
  children: ReactNode;
}) {
  return <div data-settings-column className={cn("@container/settings", SETTINGS_COLUMN)}>
    {breadcrumb && breadcrumb.length > 0 && <nav aria-label="Breadcrumb" className="mb-2 flex items-center gap-1.5 text-xs">
      {breadcrumb.map((crumb, index) => <span key={`${crumb.label}-${index}`} className="flex items-center gap-1.5">
        {index > 0 && <span aria-hidden="true" className="text-muted-foreground">/</span>}
        {crumb.onClick
          ? <Button variant="text" size="inline" onClick={crumb.onClick}>{crumb.label}</Button>
          : <span className="text-foreground">{crumb.label}</span>}
      </span>)}
    </nav>}
    <header className="mb-5 flex flex-wrap items-center gap-4">
      <div className="min-w-0 flex-1">
        <h2 className="font-display text-title font-semibold leading-tight tracking-tight text-foreground">{title}</h2>
        {description && <p className="mt-1 text-ui leading-relaxed text-muted-foreground">{description}</p>}
      </div>
      {action && <div className="flex shrink-0 items-center gap-2 pt-0.5">{action}</div>}
    </header>
    <div className="space-y-6">{children}</div>
  </div>;
}

/* ── fold ─────────────────────────────────────────────────────────────────── */

/** The page's one "Advanced" fold, closed until opened. Search opens it when
 *  a hit lives inside. */
export function SettingsFold({ label = "Advanced", children }: { label?: string; children: ReactNode }) {
  return <details data-settings-fold className="group/fold">
    <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-md px-0.5 text-ui text-muted-foreground transition-colors hover:text-foreground [&::-webkit-details-marker]:hidden">
      <CaretRight size={12} strokeWidth={1.7} aria-hidden="true" className="transition-transform group-open/fold:rotate-90" />{label}
    </summary>
    <div className="mt-4 space-y-6">{children}</div>
  </details>;
}

/* ── group ────────────────────────────────────────────────────────────────── */

/** A labelled card of rows. The label is outside the card; the card holds only
 *  rows, separated by hairlines rather than by margin. */
export function SettingsGroup({ label, note, children, className, anchor }: {
  label?: string;
  note?: ReactNode;
  children: ReactNode;
  className?: string;
  /** The search index id for this group (see settingsSearch.ts). */
  anchor?: string;
}) {
  return <section data-settings-row={anchor} className={cn(GROUP_HIGHLIGHT, className)}>
    {(label || note) && <div className="mb-2 flex flex-wrap items-baseline gap-x-3 gap-y-1 px-0.5">
      {label && <h3 className="text-ui font-medium text-foreground">{label}</h3>}
      {note && <span className="ml-auto text-meta text-muted-foreground">{note}</span>}
    </div>}
    <div className="overflow-hidden rounded-xl border border-border-card bg-card">
      <div className="divide-y divide-border">{children}</div>
    </div>
  </section>;
}

/* ── search reveal ────────────────────────────────────────────────────────── */

/** What a search hit looks like for a moment after the screen scrolls to it. */
const ROW_HIGHLIGHT = "transition-colors duration-300 data-search-highlight:bg-accent";
const GROUP_HIGHLIGHT = "scroll-mt-6 rounded-xl outline-offset-4 transition-[outline-color] duration-300 data-search-highlight:outline-2 data-search-highlight:outline-foreground/25";

/* ── row ──────────────────────────────────────────────────────────────────── */

/** The 28px lead box a harness mark or an icon sits in. */
export function RowLead({ children }: { children: ReactNode }) {
  return <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-popover text-muted-foreground">{children}</span>;
}

/**
 * One label, one control.
 *
 * `onOpen` turns the whole row into the button that opens a detail page, and
 * the row ends in a chevron instead of a control. `saved` renders the brief
 * confirmation *beside* the control: a switch that vanishes for a second and a
 * half the moment it is clicked is a switch the user cannot correct.
 */
export function SettingsRow({ label, openLabel, description, mono, lead, control, adornment, onOpen, expanded, saved, disabled, className, anchor }: {
  label: ReactNode;
  /** The accessible name of the open action, when `label` is not plain text. */
  openLabel?: string;
  description?: ReactNode;
  /** Render the description as mono meta (an id, a path, a version line). */
  mono?: boolean;
  lead?: ReactNode;
  control?: ReactNode;
  /** Non-interactive content inside a row that opens when clicked. */
  adornment?: ReactNode;
  onOpen?: () => void;
  expanded?: boolean;
  saved?: boolean;
  disabled?: boolean;
  className?: string;
  /** The search index id for this row (see settingsSearch.ts). */
  anchor?: string;
}) {
  const text = <span className="min-w-0 flex-1 basis-40 text-left">
    <span className="block text-ui text-foreground">{label}</span>
    {description !== undefined && description !== null && description !== "" && <span className={cn(
      "mt-0.5 block text-muted-foreground",
      // Prose wraps; an id or a path truncates, because half a sentence is
      // useless and half a hash is still a hash you can widen the window to read.
      mono ? "truncate font-mono text-caption" : "text-meta leading-relaxed",
    )}>{description}</span>}
  </span>;
  const trailing = <>
    {saved && <SavedFlash />}
    {adornment}
    {control}
    {onOpen && (expanded
      ? <CaretDown size={12} strokeWidth={1.7} aria-hidden="true" className="shrink-0 text-muted-foreground" />
      : <CaretRight size={12} strokeWidth={1.7} aria-hidden="true" className="shrink-0 text-muted-foreground" />)}
  </>;

  // A row that only opens a page is the button, so the hit target matches what
  // the chevron promises.
  if (onOpen && !control) {
    return <RowButton
      size="bare"
      shape="flush"
      data-settings-row={anchor}
      disabled={disabled}
      aria-label={openLabel}
      aria-expanded={expanded}
      onClick={onOpen}
      className={className}
    >
      <span className="flex min-h-11 w-full min-w-0 items-center gap-3 px-3.5 py-2.5 transition-colors duration-300 in-data-search-highlight:bg-accent">
        {lead && <RowLead>{lead}</RowLead>}{text}{trailing}
      </span>
    </RowButton>;
  }

  // A row that both opens a page and carries an action (Install next to a
  // chevron) cannot nest one button inside another, so the open action is a
  // full-bleed button underneath and the action sits above it. One accessible
  // name each, and the whole row is still the target for "open this".
  if (onOpen) {
    return <div data-settings-row={anchor} className={cn("relative flex min-h-11 flex-wrap items-center gap-3 px-3.5 py-2.5", ROW_HIGHLIGHT, className)}>
      <RowButton
        size="bare"
        shape="flush"
        disabled={disabled}
        aria-label={openLabel ?? (typeof label === "string" ? label : "Open")}
        onClick={onOpen}
        className="absolute inset-0 z-0"
      />
      {lead && <span className="pointer-events-none relative z-10"><RowLead>{lead}</RowLead></span>}
      <span className="pointer-events-none relative z-10 flex min-w-0 flex-1">{text}</span>
      <span className="relative z-10 flex shrink-0 flex-wrap items-center gap-2">{trailing}</span>
    </div>;
  }

  return <div data-settings-row={anchor} className={cn("flex min-h-11 flex-wrap items-center gap-3 px-3.5 py-2.5", ROW_HIGHLIGHT, disabled && "opacity-45", className)}>
    {lead && <RowLead>{lead}</RowLead>}{text}{trailing}
  </div>;
}

/** A row whose control needs the full width beneath the label: an editor, a
 *  textarea, a wizard step. Still a row of the same card. */
export function SettingsBlockRow({ label, description, children, action, className, anchor }: {
  label?: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  anchor?: string;
}) {
  return <div data-settings-row={anchor} className={cn("px-3.5 py-3", ROW_HIGHLIGHT, className)}>
    {(label || action) && <div className="mb-2 flex items-baseline gap-3">
      <div className="min-w-0 flex-1">
        {label && <span className="block text-ui text-foreground">{label}</span>}
        {description && <span className="mt-0.5 block text-meta text-muted-foreground">{description}</span>}
      </div>
      {action}
    </div>}
    {children}
  </div>;
}

/** The brief confirmation beside a control after the host stored its value. */
export function SavedFlash() {
  return <span role="status" className="flex shrink-0 items-center gap-1 text-caption text-success">
    <Check size={12} strokeWidth={1.7} aria-hidden="true" />Saved
  </span>;
}

/**
 * The 1.5 second confirmation a switch or select row shows after it persists.
 *
 * Keyed by row so two rows saving at once each keep their own flash, and
 * cleared on unmount so a page change cannot fire a timer into a dead tree.
 */
export function useSavedFlash(): [(key: string) => boolean, (key: string) => void] {
  const [flashed, setFlashed] = useState<Record<string, number>>({});
  const timers = useRef<number[]>([]);
  useEffect(() => () => { for (const id of timers.current) window.clearTimeout(id); }, []);
  const flash = useCallback((key: string) => {
    setFlashed(current => ({ ...current, [key]: (current[key] ?? 0) + 1 }));
    timers.current.push(window.setTimeout(() => {
      setFlashed(current => {
        const next = { ...current };
        const remaining = (next[key] ?? 1) - 1;
        if (remaining <= 0) delete next[key]; else next[key] = remaining;
        return next;
      });
    }, 1500));
  }, []);
  const isFlashed = useCallback((key: string) => (flashed[key] ?? 0) > 0, [flashed]);
  return [isFlashed, flash];
}

/* ── controls ─────────────────────────────────────────────────────────────── */

export function Switch({ checked, onChange, disabled, label }: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return <SwitchControl checked={checked} disabled={disabled} aria-label={label} onCheckedChange={onChange} />;
}

export type SelectOption = {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  lead?: ReactNode;
  /** Options sharing a group are listed under that label. */
  group?: string;
};

/**
 * A select that renders no native select element: the shared Select primitive
 * with the kit's width and option shape (lead, description, groups).
 */
export function Select({ value, options, onChange, disabled, label, placeholder = "Choose", width = "w-56" }: {
  value: string;
  options: SelectOption[];
  onChange: (next: string) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
  width?: string;
}) {
  const selected = options.find(option => option.value === value);
  const groups = [...new Set(options.map(option => option.group))];
  const item = (option: SelectOption) => <SelectItem key={option.value} value={option.value} label={option.label} disabled={option.disabled}>
    <span className="flex min-w-0 items-center gap-2">
      {option.lead}
      <span className="min-w-0 flex-1">
        <span className="block">{option.label}</span>
        {option.description && <span className="mt-0.5 block text-caption text-muted-foreground">{option.description}</span>}
      </span>
    </span>
  </SelectItem>;
  // The width sits on a wrapper: the trigger owns its own box.
  return <span className={cn(width, "inline-flex max-w-full shrink-0")}>
    <SelectRoot
      value={value}
      onValueChange={next => { if (next !== null) onChange(next); }}
      disabled={disabled || options.length === 0}
      items={options}
    >
      <SelectTrigger size="sm" aria-label={label}>
        {selected?.lead}
        <span className="min-w-0 flex-1 truncate">{selected?.label ?? (value || placeholder)}</span>
      </SelectTrigger>
      {/* Portalled, so the cards' overflow clipping cannot cut it off. */}
      <SelectPopup aria-label={label} align="end" alignItemWithTrigger={false}>
        {groups.length === 1 && groups[0] === undefined
          ? options.map(item)
          : groups.map(group => group === undefined
            ? options.filter(option => option.group === undefined).map(item)
            : <SelectGroup key={group}><SelectGroupLabel>{group}</SelectGroupLabel>{options.filter(option => option.group === group).map(item)}</SelectGroup>)}
      </SelectPopup>
    </SelectRoot>
  </span>;
}

/** A single-line text field. Same box as the select, without the chevron. */
export function Field({ value, onChange, disabled, label, placeholder, type = "text", width = "w-56", mono, onKeyDown }: {
  value: string;
  onChange: (next: string) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
  type?: "text" | "password" | "time" | "search";
  width?: string;
  mono?: boolean;
  onKeyDown?: (event: React.KeyboardEvent<HTMLInputElement>) => void;
}) {
  // The width and the mono face sit on a wrapper: the field owns its own box and type.
  return <span className={cn(width, "inline-flex max-w-full shrink-0", mono && "font-mono")}><Input
    size="sm"
    type={type}
    value={value}
    disabled={disabled}
    aria-label={label}
    placeholder={placeholder}
    autoComplete={type === "password" ? "off" : undefined}
    spellCheck={false}
    onKeyDown={onKeyDown}
    onChange={event => onChange(event.target.value)}
  /></span>;
}

/**
 * A whole-number field that saves on change. A value outside `min`..`max` is
 * marked invalid and never sent; a valid one is committed after a short pause
 * or on blur, so typing "120" does not save 1 and then 12.
 */
export function NumberField({ value, min, max, onChange, disabled, label, width = "w-24" }: {
  value: number;
  min: number;
  max: number;
  onChange: (next: number) => void;
  disabled?: boolean;
  label: string;
  width?: string;
}) {
  const [text, setText] = useState(String(value));
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => { setText(String(value)); }, [value]);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const parse = (raw: string) => {
    const parsed = raw.trim() === "" ? Number.NaN : Number(raw);
    return Number.isInteger(parsed) && parsed >= min && parsed <= max ? parsed : null;
  };
  const valid = parse(text) !== null;
  const commit = (raw: string) => {
    window.clearTimeout(timer.current);
    const parsed = parse(raw);
    if (parsed !== null && parsed !== value) onChange(parsed);
  };
  return <span className={cn(width, "inline-flex max-w-full shrink-0 tabular-nums")}><Input
    size="sm"
    type="number"
    inputMode="numeric"
    aria-label={label}
    aria-invalid={valid ? undefined : true}
    min={min}
    max={max}
    step={1}
    value={text}
    disabled={disabled}
    onChange={event => {
      const raw = event.target.value;
      setText(raw);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => commit(raw), 400);
    }}
    onBlur={event => commit(event.target.value)}
  /></span>;
}

/** A multi-line editor for a system prompt. Dirties the page; never saves. */
export function TextArea({ value, onChange, disabled, label, placeholder, rows = 8 }: {
  value: string;
  onChange: (next: string) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
  rows?: number;
}) {
  return <div className="flex font-mono"><Textarea
    value={value}
    rows={rows}
    disabled={disabled}
    aria-label={label}
    placeholder={placeholder}
    spellCheck={false}
    onChange={event => onChange(event.target.value)}
    // The primitive sizes to its content, which ignores `rows`; keep the asked height as a floor.
    style={{ minHeight: `calc(${rows}lh + 0.625rem)` }}
  /></div>;
}

/* ── buttons ──────────────────────────────────────────────────────────────── */

/** Foreground fill. Reserved for Save and Connect, and nothing else. */
export function PrimaryButton({ children, onClick, disabled, busy, type = "button" }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; type?: "button" | "submit";
  /** The action is in flight: disabled, and the label wears the skeleton sweep. */
  busy?: boolean;
}) {
  return <Button size="xs" type={type} disabled={disabled} loading={busy} onClick={onClick}>{children}</Button>;
}

/** Bordered. Install, Repair, Retry, and the wizard's step actions. */
export function GhostButton({ children, onClick, disabled, busy, ariaLabel }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; ariaLabel?: string;
  /** The action is in flight: disabled, and the label wears the skeleton sweep. */
  busy?: boolean;
}) {
  return <Button size="xs" variant="outline" aria-label={ariaLabel} disabled={disabled} loading={busy} onClick={onClick}>{children}</Button>;
}

/** Text only. `tone="destructive"` is Remove; the default muted tone is Reset. */
export function TextButton({ children, onClick, disabled, busy, tone = "muted", ariaLabel }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; busy?: boolean; tone?: "muted" | "destructive"; ariaLabel?: string;
}) {
  return <Button size="xs" variant={tone === "destructive" ? "destructive-ghost" : "quiet"} aria-label={ariaLabel} disabled={disabled} loading={busy} onClick={onClick}>{children}</Button>;
}

/* ── pills ────────────────────────────────────────────────────────────────── */

export type PillTone = "neutral" | "success" | "warning" | "info" | "destructive";

const PILL_TONE: Record<PillTone, string> = {
  neutral: "border-border text-muted-foreground",
  success: "border-success/30 bg-success/10 text-success",
  warning: "border-warning/30 bg-warning/10 text-warning",
  info: "border-info/30 bg-info/10 text-info",
  destructive: "border-destructive/30 bg-destructive/10 text-destructive",
};

/** The one place tint is allowed outside a harness mark, and only for status. */
export function StatusPill({ tone = "neutral", children }: { tone?: PillTone; children: ReactNode }) {
  return <span className={cn("shrink-0 rounded-full border px-1.5 py-0.5 text-caption leading-none", PILL_TONE[tone])}>{children}</span>;
}

/* ── save bar ─────────────────────────────────────────────────────────────── */

/**
 * The page's one save affordance.
 *
 * Every editor and text field on a page routes here, which is what replaced
 * five different save patterns (per-card Save, header Save, bottom Save,
 * autosave, and a global Reset that also deleted agents) with one.
 */
export function SaveBar({ dirty, saving, canSave = true, onSave, onDiscard, label = "Unsaved changes", saveLabel = "Save" }: {
  dirty: boolean;
  /** A request is in flight. Both actions are frozen. */
  saving: boolean;
  /** The draft is incomplete (an empty name), so Save is offered but refused. */
  canSave?: boolean;
  onSave: () => void;
  onDiscard: () => void;
  label?: string;
  saveLabel?: string;
}) {
  if (!dirty) return null;
  return <div className="sticky bottom-0 z-20 -mx-1 mt-[26px] flex flex-wrap items-center gap-3 rounded-xl border border-border-card bg-card px-3.5 py-2.5">
    <span className="min-w-0 flex-1 text-ui text-foreground">{label}</span>
    <TextButton onClick={onDiscard} disabled={saving}>Discard</TextButton>
    <PrimaryButton busy={saving} onClick={onSave} disabled={saving || !canSave}>{saveLabel}</PrimaryButton>
  </div>;
}
