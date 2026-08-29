import { useEffect, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import type { AdapterDescriptor, Harness } from "../types";
import { harnessLabel } from "../utils";

// OpenCode Zen's free tier suffixes its catalog labels with "(Unlimited)".
// It names the plan, not the model, so it is noise wherever Bridge shows the
// label as a name — the pill and the picker rows alike.
const UNLIMITED_SUFFIX = /\s*\(unlimited\)\s*$/i;

/** Exact-match lookup of a session's configured model, for display outside
 *  the control itself (e.g. a tooltip or a plain-text mention). */
export function modelDisplayName(adapters: AdapterDescriptor[], harness: Harness, model?: string | null): string {
  const adapter = adapters.find(item => item.id === harness);
  const label = adapter?.models.find(option => option.id === model)?.label ?? model ?? "Automatic";
  return label.replace(UNLIMITED_SUFFIX, "");
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
};

export function ChatModelControl({ adapters, harness, model, disabled, disabledReason, onChange, compact, roleLabel = "Chat", maxWidthClassName = "max-w-[220px]", placement = "up" }: ChatModelControlProps) {
  const [open, setOpen] = useState(false);
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
  const chatAdapters = adapters.filter(adapter => ["codex", "claude", "opencode"].includes(adapter.id));
  const current = chatAdapters.find(adapter => adapter.id === harness);
  const currentModel = current?.models.find(option => option.id === model) ?? current?.models.find(option => option.defaultForTier) ?? current?.models[0];
  const modelLabel = (currentModel?.label ?? model ?? "Default").replace(UNLIMITED_SUFFIX, "");
  // Some catalogs bake the provider into the model label ("OpenCode Go ·
  // MiMo V2.5"), and prefixing the harness again read as a stutter:
  // "OpenCode · OpenCode Go · MiMo V2.5". When the model label already opens
  // with the harness label, it carries the whole pill by itself.
  const harnessName = harnessLabel(harness);
  const compactLabel = modelLabel.toLowerCase().startsWith(harnessName.toLowerCase())
    ? modelLabel
    : `${harnessName} · ${modelLabel}`;
  return <div className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className={`flex ${maxWidthClassName} items-center gap-1 rounded-full transition-colors disabled:opacity-45 ${compact ? "h-8 px-2 text-xs text-muted-foreground hover:bg-accent" : "h-[28px] px-2 text-[11.5px] text-foreground/90 hover:bg-accent"}`} title={disabled ? disabledReason ?? "Model selection is temporarily unavailable" : compactLabel} aria-label={`${roleLabel} model: ${harnessLabel(harness)} ${modelLabel}`}>
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span>
      <ChevronDown size={compact ? 14 : 12} className={`shrink-0 text-muted-foreground/55 transition-transform ${open ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div className={`u-glass-popover absolute left-0 z-40 w-[280px] py-1.5 rounded-2xl max-h-[340px] overflow-y-auto ${placement === "down" ? "top-full mt-2" : "bottom-full mb-2"}`}>
        <div className="border-b border-border/60 px-3 pb-2 pt-1">
          <p className="text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground/65">{roleLabel} runtime</p>
          <p className="mt-1 text-[10px] leading-4 text-muted-foreground/55">Switching starts a fresh provider session. The chat stays visible, but provider reasoning state resets.</p>
        </div>
        {chatAdapters.map((adapter, index) => <div key={adapter.id} className={index > 0 ? "mt-1 pt-1 border-t border-border/60" : ""}>
          <div className="px-3 py-1.5 text-[9px] font-semibold tracking-[0.12em] uppercase text-muted-foreground/50 flex items-center gap-2">
            <span>{adapter.label}</span>
            {!adapter.available && <span className="normal-case tracking-normal font-normal text-muted-foreground/40">unavailable</span>}
          </div>
          {(adapter.models.length ? adapter.models : [{ id: "", label: "Default", tier: "fast" as const, defaultForTier: true }]).map(option => {
            const selected = adapter.id === harness && (option.id ? option.id === model : !model);
            return <button key={`${adapter.id}:${option.id || "default"}`} type="button" disabled={!adapter.available} onClick={() => { onChange(adapter.id as Harness, option.id || null); setOpen(false); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors disabled:opacity-40 ${selected ? "bg-foreground/[0.08]" : "hover:bg-foreground/[0.05]"}`}>
              <span className="flex-1 min-w-0 text-[12.5px] text-foreground whitespace-nowrap overflow-hidden text-ellipsis">{option.label.replace(UNLIMITED_SUFFIX, "")}</span>
              <span className="text-[9.5px] uppercase tracking-[0.06em] text-muted-foreground/45">{option.tier}</span>
              {selected && <Check size={13} className="text-foreground/80" aria-hidden="true" />}
            </button>;
          })}
        </div>)}
      </div>
    </>}
  </div>;
}
