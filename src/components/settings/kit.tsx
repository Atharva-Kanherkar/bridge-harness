// The Settings design system, in one file.
//
// The rule every page here obeys: **a page is a column of groups, a group is a
// card of rows, a row is one label and one control.** Nothing else appears on a
// settings page. Before this kit existed, eight sections had four content
// widths, ten type sizes, five card styles, and five different save patterns,
// because each one drew its own layout. A page that can only compose these
// primitives cannot drift that way again.
//
// Sizes are fixed here rather than chosen per page: controls are 28px tall with
// 12px text and an 8px radius; a row is at least 44px with 10px/14px padding;
// a group card is 12px radius with a hairline between rows and 26px of air
// after it. Type is limited to 17 / 13 / 12 / 11.5 / 11 and 10.5 mono.
//
// Icons are Phosphor Regular. No symbol on the Settings screen comes from
// `lucide-react`.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { CaretDown, CaretRight, Check } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";

/* ── page ─────────────────────────────────────────────────────────────────── */

export type Crumb = { label: string; onClick?: () => void };

/**
 * One settings page: an optional breadcrumb, a title, one line of description,
 * an optional action, then the groups.
 *
 * The column width lives here and only here, which is what keeps the content
 * from jumping as the rail selection changes.
 */
export function SettingsPage({ title, description, breadcrumb, action, children }: {
  title: string;
  description?: string;
  breadcrumb?: Crumb[];
  action?: ReactNode;
  children: ReactNode;
}) {
  return <div data-settings-column className="mx-auto w-full max-w-[720px] px-6 pb-24 pt-[30px]">
    {breadcrumb && breadcrumb.length > 0 && <nav aria-label="Breadcrumb" className="mb-2 flex items-center gap-1.5 text-xs">
      {breadcrumb.map((crumb, index) => <span key={`${crumb.label}-${index}`} className="flex items-center gap-1.5">
        {index > 0 && <span aria-hidden="true" className="text-muted-foreground/50">/</span>}
        {crumb.onClick
          ? <button type="button" onClick={crumb.onClick} className="rounded text-muted-foreground transition-colors hover:text-foreground">{crumb.label}</button>
          : <span className="text-foreground">{crumb.label}</span>}
      </span>)}
    </nav>}
    <header className="mb-6 flex items-start gap-4">
      <div className="min-w-0 flex-1">
        <h2 className="text-[17px] font-semibold leading-tight text-foreground">{title}</h2>
        {description && <p className="mt-1 text-xs text-muted-foreground">{description}</p>}
      </div>
      {action && <div className="flex shrink-0 items-center gap-2 pt-0.5">{action}</div>}
    </header>
    <div className="space-y-[26px]">{children}</div>
  </div>;
}

/* ── group ────────────────────────────────────────────────────────────────── */

/** A labelled card of rows. The label is outside the card; the card holds only
 *  rows, separated by hairlines rather than by margin. */
export function SettingsGroup({ label, note, children, className }: {
  label?: string;
  note?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return <section className={className}>
    {(label || note) && <div className="mb-2 flex items-baseline gap-3 px-0.5">
      {label && <h3 className="text-[11px] font-semibold uppercase tracking-[0.09em] text-muted-foreground">{label}</h3>}
      {note && <span className="ml-auto text-[11px] text-muted-foreground/70">{note}</span>}
    </div>}
    <div className="overflow-hidden rounded-xl border border-border-card bg-card">
      <div className="divide-y divide-border">{children}</div>
    </div>
  </section>;
}

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
export function SettingsRow({ label, openLabel, description, mono, lead, control, onOpen, saved, disabled, className }: {
  label: ReactNode;
  /** The accessible name of the open action, when `label` is not plain text. */
  openLabel?: string;
  description?: ReactNode;
  /** Render the description as mono meta (an id, a path, a version line). */
  mono?: boolean;
  lead?: ReactNode;
  control?: ReactNode;
  onOpen?: () => void;
  saved?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const text = <span className="min-w-0 flex-1 text-left">
    <span className="block truncate text-[13px] text-foreground">{label}</span>
    {description !== undefined && description !== null && description !== "" && <span className={cn(
      "mt-0.5 block truncate text-muted-foreground",
      mono ? "font-mono text-[10.5px]" : "text-[11.5px]",
    )}>{description}</span>}
  </span>;
  const trailing = <>
    {saved && <SavedFlash />}
    {control}
    {onOpen && <CaretRight size={12} weight="regular" aria-hidden="true" className="shrink-0 text-muted-foreground/60" />}
  </>;

  // A row that only opens a page is the button, so the hit target matches what
  // the chevron promises.
  if (onOpen && !control) {
    return <button
      type="button"
      disabled={disabled}
      aria-label={openLabel}
      onClick={onOpen}
      className={cn("flex min-h-11 w-full items-center gap-3 px-3.5 py-2.5 text-left transition-colors hover:bg-accent disabled:opacity-45", className)}
    >{lead && <RowLead>{lead}</RowLead>}{text}{trailing}</button>;
  }

  // A row that both opens a page and carries an action (Install next to a
  // chevron) cannot nest one button inside another, so the open action is a
  // full-bleed button underneath and the action sits above it. One accessible
  // name each, and the whole row is still the target for "open this".
  if (onOpen) {
    return <div className={cn("relative flex min-h-11 items-center gap-3 px-3.5 py-2.5", className)}>
      <button
        type="button"
        disabled={disabled}
        aria-label={openLabel ?? (typeof label === "string" ? label : "Open")}
        onClick={onOpen}
        className="absolute inset-0 z-0 transition-colors hover:bg-accent disabled:opacity-45"
      />
      {lead && <span className="pointer-events-none relative z-10"><RowLead>{lead}</RowLead></span>}
      <span className="pointer-events-none relative z-10 flex min-w-0 flex-1">{text}</span>
      <span className="relative z-10 flex shrink-0 items-center gap-2">{trailing}</span>
    </div>;
  }

  return <div className={cn("flex min-h-11 items-center gap-3 px-3.5 py-2.5", disabled && "opacity-45", className)}>
    {lead && <RowLead>{lead}</RowLead>}{text}{trailing}
  </div>;
}

/** A row whose control needs the full width beneath the label: an editor, a
 *  textarea, a wizard step. Still a row of the same card. */
export function SettingsBlockRow({ label, description, children, action, className }: {
  label?: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return <div className={cn("px-3.5 py-3", className)}>
    {(label || action) && <div className="mb-2 flex items-baseline gap-3">
      <div className="min-w-0 flex-1">
        {label && <span className="block text-[13px] text-foreground">{label}</span>}
        {description && <span className="mt-0.5 block text-[11.5px] text-muted-foreground">{description}</span>}
      </div>
      {action}
    </div>}
    {children}
  </div>;
}

function SavedFlash() {
  return <span role="status" className="flex shrink-0 items-center gap-1 text-[11px] text-success">
    <Check size={12} weight="regular" aria-hidden="true" />Saved
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

/** 32x18. On is a foreground track with a background knob; there is no hue. */
export function Switch({ checked, onChange, disabled, label }: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return <button
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    disabled={disabled}
    onClick={() => onChange(!checked)}
    className={cn(
      "relative h-[18px] w-8 shrink-0 rounded-full transition-colors disabled:opacity-40",
      checked ? "bg-foreground" : "bg-foreground/15",
    )}
  >
    <span className={cn(
      "absolute top-[2px] size-[14px] rounded-full transition-[left] duration-150",
      checked ? "left-4 bg-background" : "left-[2px] bg-background",
    )} />
  </button>;
}

export type SelectOption = {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  lead?: ReactNode;
};

const CONTROL = "h-7 rounded-lg border border-border-card bg-popover px-2.5 text-xs text-foreground outline-none transition-colors disabled:opacity-45";

/**
 * A select that renders no native select element.
 *
 * Native selects render a macOS system menu that ignores every token in this
 * file, which is why the design bans them outright. This is a button plus a
 * listbox: the same keyboard contract, drawn in Bridge's own chrome.
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
  const [open, setOpen] = useState(false);
  const selected = options.find(option => option.value === value);
  return <div className="relative shrink-0">
    <button
      type="button"
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={label}
      disabled={disabled || options.length === 0}
      onClick={() => setOpen(current => !current)}
      className={cn(CONTROL, width, "flex max-w-full items-center gap-1.5 hover:bg-accent")}
    >
      {selected?.lead}
      <span className="min-w-0 flex-1 truncate text-left">{selected?.label ?? (value || placeholder)}</span>
      <CaretDown size={12} weight="regular" aria-hidden="true" className={cn("shrink-0 text-muted-foreground/60 transition-transform", open && "rotate-180")} />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div
        role="listbox"
        aria-label={label}
        className="u-glass-popover absolute right-0 top-full z-40 mt-1 max-h-72 min-w-full overflow-y-auto rounded-lg border border-border bg-popover p-1"
      >
        {options.map(option => <button
          key={option.value}
          type="button"
          role="option"
          aria-selected={option.value === value}
          disabled={option.disabled}
          onClick={() => { onChange(option.value); setOpen(false); }}
          className={cn(
            "flex w-full items-center gap-2 whitespace-nowrap rounded-md px-2 py-1.5 text-left text-xs transition-colors disabled:opacity-40",
            option.value === value ? "bg-accent text-foreground" : "text-foreground hover:bg-accent",
          )}
        >
          {option.lead}
          <span className="min-w-0 flex-1">
            {option.label}
            {option.description && <span className="mt-0.5 block text-[10.5px] text-muted-foreground">{option.description}</span>}
          </span>
          {option.value === value && <Check size={12} weight="regular" aria-hidden="true" className="shrink-0" />}
        </button>)}
      </div>
    </>}
  </div>;
}

/** A single-line text field. Same box as the select, without the chevron. */
export function Field({ value, onChange, disabled, label, placeholder, type = "text", width = "w-56", mono, onKeyDown }: {
  value: string;
  onChange: (next: string) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
  type?: "text" | "password";
  width?: string;
  mono?: boolean;
  onKeyDown?: (event: React.KeyboardEvent<HTMLInputElement>) => void;
}) {
  return <input
    type={type}
    value={value}
    disabled={disabled}
    aria-label={label}
    placeholder={placeholder}
    autoComplete={type === "password" ? "off" : undefined}
    spellCheck={false}
    onKeyDown={onKeyDown}
    onChange={event => onChange(event.target.value)}
    className={cn(CONTROL, width, "shrink-0 placeholder:text-muted-foreground/70 focus:border-foreground/25", mono && "font-mono text-[11px]")}
  />;
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
  return <textarea
    value={value}
    rows={rows}
    disabled={disabled}
    aria-label={label}
    placeholder={placeholder}
    spellCheck={false}
    onChange={event => onChange(event.target.value)}
    className="w-full resize-y rounded-lg border border-border-card bg-popover px-2.5 py-2 font-mono text-[11px] leading-relaxed text-foreground outline-none transition-colors placeholder:text-muted-foreground/70 focus:border-foreground/25 disabled:opacity-45"
  />;
}

/* ── buttons ──────────────────────────────────────────────────────────────── */

const BUTTON = "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-xs transition-colors disabled:opacity-40";

/** Foreground fill. Reserved for Save and Connect, and nothing else. */
export function PrimaryButton({ children, onClick, disabled, type = "button" }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; type?: "button" | "submit";
}) {
  return <button type={type} disabled={disabled} onClick={onClick} className={cn(BUTTON, "bg-foreground font-medium text-background hover:opacity-90")}>{children}</button>;
}

/** Bordered. Install, Repair, Retry, and the wizard's step actions. */
export function GhostButton({ children, onClick, disabled, ariaLabel }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; ariaLabel?: string;
}) {
  return <button type="button" aria-label={ariaLabel} disabled={disabled} onClick={onClick} className={cn(BUTTON, "border border-border-card bg-popover text-foreground hover:bg-accent")}>{children}</button>;
}

/** Text only. `tone="destructive"` is Remove; the default muted tone is Reset. */
export function TextButton({ children, onClick, disabled, tone = "muted", ariaLabel }: {
  children: ReactNode; onClick?: () => void; disabled?: boolean; tone?: "muted" | "destructive"; ariaLabel?: string;
}) {
  return <button
    type="button"
    aria-label={ariaLabel}
    disabled={disabled}
    onClick={onClick}
    className={cn(BUTTON, tone === "destructive" ? "text-destructive hover:bg-destructive/10" : "text-muted-foreground hover:bg-accent hover:text-foreground")}
  >{children}</button>;
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
  return <span className={cn("shrink-0 rounded-full border px-1.5 py-0.5 text-[10.5px] leading-none", PILL_TONE[tone])}>{children}</span>;
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
  return <div className="sticky bottom-0 z-20 -mx-1 mt-[26px] flex items-center gap-3 rounded-xl border border-border-card bg-card px-3.5 py-2.5">
    <span className="min-w-0 flex-1 text-[13px] text-foreground">{label}</span>
    <TextButton onClick={onDiscard} disabled={saving}>Discard</TextButton>
    <PrimaryButton onClick={onSave} disabled={saving || !canSave}>{saving ? "Saving…" : saveLabel}</PrimaryButton>
  </div>;
}
