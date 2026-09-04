import { useEffect, useState } from "react";
import { Check, ChevronDown, Search } from "lucide-react";
import type { AdapterDescriptor, Harness } from "../types";
import { harnessLabel } from "../utils";
import { HarnessMark } from "./harnessMarks";
import { cn } from "@/lib/utils";

// Reasoning effort, in the order the segmented control shows them. Values match
// the wire `Effort` union ("low" | "medium" | "high" | "xhigh"); the labels are
// the compact glyphs the 2a footer draws.
const EFFORT_OPTIONS: { value: string; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Med" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
];

// Vendor noise at the tail of a catalog label. OpenCode Zen's free tier
// suffixes "(Unlimited)": it names the plan, not the model. Cursor reports a
// model whose variant brackets came through empty ("default[]"), which read as
// a rendering bug in the pill. Both are noise wherever Bridge shows the label
// as a name — the pill, the picker rows, and modelDisplayName alike. Repeated
// so a label carrying both, in either order, comes out clean.
const LABEL_NOISE_SUFFIX = /(?:\s*(?:\(unlimited\)|\[\s*\]))+\s*$/i;

/** Display-only cleanup of a catalog label. Ids and what `onChange` emits are
 *  never touched — only the text Bridge renders. */
export function cleanModelLabel(label: string): string {
  const cleaned = label.replace(LABEL_NOISE_SUFFIX, "").trim();
  // A label that is nothing but noise still has to name something.
  return cleaned || label.trim();
}

/** Exact-match lookup of a session's configured model, for display outside
 *  the control itself (e.g. a tooltip or a plain-text mention). */
export function modelDisplayName(adapters: AdapterDescriptor[], harness: Harness, model?: string | null): string {
  const adapter = adapters.find(item => item.id === harness);
  const label = adapter?.models.find(option => option.id === model)?.label ?? model ?? "Automatic";
  return cleanModelLabel(label);
}

export type ChatModelControlProps = {
  adapters: AdapterDescriptor[];
  harness: Harness;
  model: string | null;
  disabled?: boolean;
  disabledReason?: string;
  onChange: (harness: Harness, model: string | null) => void;
  compact?: boolean;
  roleLabel?: string;
  /** Overrides the pill's width cap. Defaults to the composer's width; the
   *  session toolbar's tighter row passes a narrower cap. */
  maxWidthClassName?: string;
  /** Which way the picker opens. The composer sits at the bottom of the view,
   *  so it opens upward — the default. A caller in the session toolbar (the
   *  top row of a container that clips its overflow) must pass `"down"`, or
   *  the panel lands above the viewport and never becomes visible. */
  placement?: "up" | "down";
  /** Active reasoning effort for the session. Highlighted in the popover's
   *  footer segmented control — the 2a home for effort, moved off the composer's
   *  old standalone pill. */
  effort?: string | null;
  /** Changes the reasoning effort. When omitted the footer control still shows
   *  the current effort but is inert — the state lives with the caller, so a
   *  surface that has not wired a setter yet reads rather than writes. */
  onEffortChange?: (effort: string) => void;
};

export function ChatModelControl({ adapters, harness, model, disabled, disabledReason, onChange, compact, roleLabel = "Chat", maxWidthClassName = "max-w-[220px]", placement = "up", effort, onEffortChange }: ChatModelControlProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // Escape closes the picker and nothing else. Captured on window so it wins
  // against modal hosts with their own window-level Escape (the aside panel
  // closes itself on Escape — without this, dismissing the picker tore down
  // the whole aside).
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [open]);
  // The backend descriptor is the authority. A provider is a chat runtime when
  // it advertises messages or a model catalog; adding a new adapter must not
  // require another frontend provider-name allowlist.
  const chatAdapters = adapters.filter(adapter => adapter.capabilities.includes("messages") || adapter.models.length > 0);
  const current = chatAdapters.find(adapter => adapter.id === harness);
  const currentModel = current?.models.find(option => option.id === model) ?? current?.models.find(option => option.defaultForTier) ?? current?.models[0];
  const modelLabel = cleanModelLabel(currentModel?.label ?? model ?? "Default");
  // Some catalogs bake the provider into the model label ("OpenCode Go ·
  // MiMo V2.5"), and prefixing the harness again read as a stutter:
  // "OpenCode · OpenCode Go · MiMo V2.5". When the model label already opens
  // with the harness label, it carries the whole pill by itself.
  const harnessName = harnessLabel(harness);
  const compactLabel = modelLabel.toLowerCase().startsWith(harnessName.toLowerCase())
    ? modelLabel
    : `${harnessName} · ${modelLabel}`;
  // Effort has a home in the footer whenever there is an effort to show or a
  // setter to drive it — otherwise the segmented control would be dead chrome.
  const showEffort = effort != null || !!onEffortChange;
  const q = query.trim().toLowerCase();
  const matchesQuery = (label: string) => !q || cleanModelLabel(label).toLowerCase().includes(q);
  return <div className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className={cn("flex items-center gap-1.5 rounded-full transition-colors disabled:opacity-45", maxWidthClassName, compact ? "h-8 px-2 text-xs text-muted-foreground hover:bg-accent" : "h-[28px] px-2 text-[11.5px] text-foreground/90 hover:bg-accent")} title={disabled ? disabledReason ?? "Model selection is temporarily unavailable" : compactLabel} aria-label={`${roleLabel} model: ${harnessLabel(harness)} ${modelLabel}`}>
      <HarnessMark harness={harness} size={14} />
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span>
      <ChevronDown size={compact ? 14 : 12} className={`shrink-0 text-muted-foreground/55 transition-transform ${open ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div className={cn("u-glass-popover absolute left-0 z-40 flex w-[340px] max-h-[420px] flex-col overflow-hidden rounded-xl border border-border bg-popover", placement === "down" ? "top-full mt-2" : "bottom-full mb-2")}>
        {/* Search header: magnifier + input, with a faint keyboard hint at the right. */}
        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
          <Search size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            type="text"
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder="Search models"
            aria-label="Search models"
            className="min-w-0 flex-1 bg-transparent text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          <span className="shrink-0 font-mono text-[11px] text-faint" aria-hidden="true">↑↓ ⏎</span>
        </div>
        <div role="listbox" aria-label={`${roleLabel} models`} className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {chatAdapters
            .map(adapter => {
              const catalog = adapter.models.length ? adapter.models : [{ id: "", label: "Default", tier: "fast" as const, defaultForTier: true }];
              const groupMatches = matchesQuery(adapter.label);
              const models = catalog.filter(option => groupMatches || matchesQuery(option.label));
              return { adapter, models };
            })
            .filter(group => group.models.length > 0)
            .map(({ adapter, models }, index) => (
              <div key={adapter.id}>
                {/* Group header: a small-caps label, quiet enough to read as a
                    section marker rather than another row competing with the
                    models beneath it. An unavailable harness is dimmed and
                    flagged, its rows shown but greyed. */}
                <div className={cn("flex items-center gap-1.5 px-2 pb-1.5 pt-2", index > 0 && "mt-1 border-t border-border/60")}>
                  <HarnessMark harness={adapter.id} size={11} className={cn("opacity-70", !adapter.available && "opacity-30")} />
                  <span className={cn("text-[9.5px] font-medium uppercase tracking-[0.12em]", adapter.available ? "text-muted-foreground/70" : "text-muted-foreground/40")}>{adapter.label}</span>
                  {!adapter.available && <span className="ml-auto text-[10px] text-muted-foreground/40">unavailable</span>}
                </div>
                {models.map(option => {
                  const selected = adapter.id === harness && (option.id ? option.id === model : !model);
                  return <button key={`${adapter.id}:${option.id || "default"}`} type="button" role="option" aria-selected={selected} disabled={!adapter.available} onClick={() => { onChange(adapter.id as Harness, option.id || null); setOpen(false); }} className={cn("flex h-9 w-full items-center gap-2 rounded-[7px] px-2 text-left transition-colors disabled:opacity-40", selected ? "bg-accent" : "hover:bg-accent", !adapter.available && "opacity-60")}>
                    <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[12.5px] text-foreground">{cleanModelLabel(option.label)}</span>
                    <span className={cn("shrink-0 font-mono text-[10px] uppercase tracking-[0.08em]", option.tier === "strong" ? "text-foreground/70" : option.tier === "standard" ? "text-muted-foreground" : "text-muted-foreground/45")}>{option.tier}</span>
                    {selected && <Check size={13} className="shrink-0 text-foreground" aria-hidden="true" />}
                  </button>;
                })}
              </div>
            ))}
        </div>
        {showEffort && (
          <div className="shrink-0 border-t border-border px-3 py-2.5" data-testid="effort-control">
            <div className="flex items-center gap-2">
              <span className="shrink-0 text-[11px] text-muted-foreground">Effort</span>
              <div role="group" aria-label="Reasoning effort" className="u-segmented flex-1">
                {EFFORT_OPTIONS.map(option => {
                  const active = effort === option.value;
                  return <button
                    key={option.value}
                    type="button"
                    aria-pressed={active}
                    disabled={!onEffortChange}
                    onClick={() => onEffortChange?.(option.value)}
                    data-effort={option.value}
                    data-active={active}
                    className="u-segmented-item flex-1 disabled:cursor-default"
                  >{option.label}</button>;
                })}
              </div>
            </div>
          </div>
        )}
        <div className="shrink-0 border-t border-border px-3 py-2.5">
          <p className="text-[12px] leading-4 text-faint">Switching restarts the provider session. History stays.</p>
        </div>
      </div>
    </>}
  </div>;
}
