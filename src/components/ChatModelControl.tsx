import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, ChevronDown, RefreshCw, Search } from "lucide-react";
import type { AdapterDescriptor, Harness } from "../types";
import { harnessLabel } from "../utils";
import { HarnessMark } from "./harnessMarks";
import { useEffortSelectorStyle } from "../theme";
import { effortLevelsFrom } from "./effort/effortLevels";
import { EffortSlider } from "./effort/EffortSlider";
import { EffortSentence } from "./effort/EffortSentence";
import { EffortList } from "./effort/EffortList";
import { cn } from "@/lib/utils";

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
  /** Re-probes provider-owned catalogues and repaints from fresh descriptors. */
  onRefresh?: () => Promise<void> | void;
};

/** The popover's tallest and shortest useful heights, in CSS pixels. */
const MAX_POPOVER_HEIGHT = 420;
const MIN_POPOVER_HEIGHT = 240;
const VIEWPORT_MARGIN = 12;

export function ChatModelControl({ adapters, harness, model, disabled, disabledReason, onChange, compact, roleLabel = "Chat", maxWidthClassName = "max-w-[220px]", placement = "up", effort, onEffortChange, onRefresh }: ChatModelControlProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  // Optimistic selection: the row the user just picked highlights and drives
  // the effort ladder at once, while the host is still switching. Cleared when
  // the props catch up or when the host's disable window ends.
  const [pending, setPending] = useState<{ harness: Harness; model: string | null } | null>(null);
  // The level the user last chose here, until the host confirms it. `sent`
  // is false while the host is still switching models — the level waits and
  // goes out the instant the switch settles, provided the pick landed. The
  // shown value is this one, so a round-trip never flashes the stale prop.
  const [queued, setQueued] = useState<{ value: string; sent: boolean } | null>(null);
  // True while a request the picker itself made — a model pick or a level —
  // is outstanding at the host. A live session's handler sets busy, which
  // flips `disabled` for the length of the request; that flip must not count
  // as "a turn started" and close the popover, must keep the effort control
  // live, and must block a second pick that would race the first on the
  // session lock. It is state of its own rather than "is there a pending
  // pick" because the props can confirm the new model while the host is still
  // busy — the highlight is settled then, the request is not. A disable from
  // anywhere else closes the popover and drops anything of ours in flight,
  // since the host is busy with something else now.
  const [inFlight, setInFlight] = useState(false);
  const wasDisabled = useRef(!!disabled);
  useEffect(() => {
    const was = wasDisabled.current;
    wasDisabled.current = !!disabled;
    if (disabled && !was) {
      if (!inFlight) { setOpen(false); setPending(null); setQueued(null); }
      return;
    }
    if (!disabled && was) {
      setInFlight(false);
      // The request settled. Only a pick the host adopted may carry the queue;
      // a failed switch must not apply the level to the model just left.
      const landed = !pending || (pending.harness === harness && pending.model === model);
      setPending(null);
      if (!queued) return;
      if (!queued.sent) {
        if (landed && onEffortChange) {
          setInFlight(true);
          setQueued({ value: queued.value, sent: true });
          onEffortChange(queued.value);
        } else {
          setQueued(null);
        }
      } else if (effort !== queued.value) {
        // The host settled without adopting the level: show what it has.
        setQueued(null);
      }
    }
  }, [disabled, inFlight, pending, queued, harness, model, effort, onEffortChange]);
  // The props caught up with the optimistic state. A host that applied the
  // change without ever going busy (the Welcome draft) has nothing of ours
  // outstanding any more; one still busy is confirming early, and stays in
  // flight until it settles.
  useEffect(() => {
    if (!pending || pending.harness !== harness || pending.model !== model) return;
    setPending(null);
    if (!disabled) setInFlight(false);
  }, [harness, model, pending, disabled]);
  useEffect(() => {
    if (!queued?.sent || effort !== queued.value) return;
    setQueued(null);
    if (!disabled) setInFlight(false);
  }, [effort, queued, disabled]);
  // Closing while nothing is in flight forgets the optimistic state. Closing
  // mid-request keeps it: the pick already went to the host, and a level chosen
  // meanwhile still goes out when the switch settles.
  useEffect(() => {
    if (!open && !disabled) { setPending(null); setQueued(null); }
  }, [open, disabled]);
  // Fit the popover to the window. The composer sits near the bottom of the
  // view, so an upward popover of fixed height ran off the top of a short
  // window; measure the room on each side, cap the height to it, and flip when
  // the requested side is too short and the other has more.
  const wrapper = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState<{ side: "up" | "down"; maxHeight: number } | null>(null);
  useLayoutEffect(() => {
    if (!open) { setFit(null); return; }
    const anchor = wrapper.current;
    if (!anchor) return;
    const measure = () => {
      const rect = anchor.getBoundingClientRect();
      // No box (jsdom, detached) means nothing to fit to: keep the defaults.
      if (rect.width === 0 && rect.height === 0) { setFit(null); return; }
      let top = 0;
      let bottom = window.innerHeight;
      // A viewport-sized popover can still be hidden behind a toolbar or a
      // scrolling pane. Intersect every clipping ancestor before choosing a side.
      for (let parent = anchor.parentElement; parent; parent = parent.parentElement) {
        const overflow = getComputedStyle(parent);
        if (/(auto|scroll|hidden|clip)/.test(overflow.overflowY || overflow.overflow)) {
          const bounds = parent.getBoundingClientRect();
          top = Math.max(top, bounds.top);
          bottom = Math.min(bottom, bounds.bottom);
        }
      }
      const above = Math.max(0, rect.top - top - VIEWPORT_MARGIN);
      const below = Math.max(0, bottom - rect.bottom - VIEWPORT_MARGIN);
      let side = placement;
      const room = side === "up" ? above : below;
      const other = side === "up" ? below : above;
      if (room < MIN_POPOVER_HEIGHT && other > room) side = side === "up" ? "down" : "up";
      // The preferred minimum is a reason to flip, never a reason to overflow.
      setFit({ side, maxHeight: Math.min(MAX_POPOVER_HEIGHT, side === "up" ? above : below) });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(anchor);
    for (let parent = anchor.parentElement; parent; parent = parent.parentElement) observer?.observe(parent);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open, placement]);
  const side = fit?.side ?? placement;
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
  // What the picker treats as selected: the optimistic pick while the host
  // switches, else the host's own model.
  const activeHarness = pending?.harness ?? harness;
  const activeModel = pending ? pending.model : model;
  const current = chatAdapters.find(adapter => adapter.id === activeHarness);
  const currentModel = current?.models.find(option => option.id === (activeModel ?? current.defaultModel));
  const modelLabel = cleanModelLabel(currentModel?.label ?? activeModel ?? "Default");
  // Some catalogs bake the provider into the model label ("OpenCode Go ·
  // MiMo V2.5"), and prefixing the harness again read as a stutter:
  // "OpenCode · OpenCode Go · MiMo V2.5". When the model label already opens
  // with the harness label, it carries the whole pill by itself.
  const harnessName = harnessLabel(activeHarness);
  const compactLabel = modelLabel.toLowerCase().startsWith(harnessName.toLowerCase())
    ? modelLabel
    : `${harnessName} · ${modelLabel}`;
  const effortLevels = effortLevelsFrom(currentModel?.supportedEffortLevels);
  // A surface that wires neither the value nor a setter does not do effort at
  // all; one that wires either shows the current level, editable or not.
  const effortSurface = effort != null || !!onEffortChange;
  const showEffort = effortLevels.length > 0 && effortSurface;
  // The user's chosen form for the control (Settings › Appearance). The list
  // style turns the popover into two panes, so a model can be picked and its
  // effort set without the popover closing in between.
  const effortStyle = useEffortSelectorStyle();
  const twoPane = effortStyle === "list" && effortSurface;
  // While the host switches to the picked model the ladder is the new model's;
  // the session's level carries over only if that ladder has it, which is the
  // backend's rule too. A level chosen here wins over both until confirmed.
  const effortValue = queued?.value ?? (pending ? (effortLevels.some(level => level.value === effort) ? effort : null) : effort);
  const changeEffort = (value: string) => {
    if (!onEffortChange) return;
    if (disabled) { setQueued({ value, sent: false }); return; }
    setInFlight(true);
    setQueued({ value, sent: true });
    onEffortChange(value);
  };
  // The picker's own request is outstanding and the host is busy with it.
  // Rows ignore a second pick meanwhile — it would race the first on the
  // session lock — and the effort control stays live so both can be set in
  // one open. Only a disable from elsewhere makes the control inert.
  const switchInFlight = !!disabled && inFlight;
  const effortDisabled = !!disabled && !inFlight;
  const effortProps = { levels: effortLevels, value: effortValue, onChange: onEffortChange ? changeEffort : undefined, disabled: effortDisabled, harness: activeHarness, modelLabel };
  const pickModel = (nextHarness: Harness, nextModel: string | null) => {
    if (switchInFlight) return;
    setInFlight(true);
    if (effortSurface) setPending({ harness: nextHarness, model: nextModel });
    onChange(nextHarness, nextModel);
    // A surface that does effort keeps the popover open so thinking can be set
    // for the new model in the same open; a plain picker closes on pick.
    if (!effortSurface) setOpen(false);
  };
  const q = query.trim().toLowerCase();
  const matchesQuery = (label: string) => !q || cleanModelLabel(label).toLowerCase().includes(q);
  return <div ref={wrapper} className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className={cn("flex min-w-0 items-center gap-1.5 rounded-md transition-colors disabled:cursor-default", maxWidthClassName, compact ? "h-8 px-2 text-xs text-muted-foreground enabled:hover:bg-accent" : "h-[28px] px-2 text-[12px] text-foreground/90 enabled:hover:bg-accent")} title={disabled ? disabledReason ?? "Model selection is temporarily unavailable" : compactLabel} aria-expanded={open} aria-haspopup="dialog" aria-label={`${roleLabel} model: ${harnessLabel(activeHarness)} ${modelLabel}`}>
      <HarnessMark harness={activeHarness} size={14} />
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span>
      <ChevronDown size={compact ? 14 : 12} className={cn("shrink-0 text-muted-foreground transition-transform", open && "rotate-180", disabled && "opacity-40")} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div
        role="dialog" aria-label="Choose a model"
        className={cn("@container/model-picker u-glass-popover absolute left-0 z-40 flex max-h-[420px] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl border border-border bg-popover", twoPane ? "w-[460px]" : "w-[340px]", side === "down" ? "top-full mt-2" : "bottom-full mb-2", fit && fit.maxHeight < MIN_POPOVER_HEIGHT && "overflow-y-auto")}
        style={fit ? { maxHeight: fit.maxHeight } : undefined}
        onKeyDown={event => {
          // Two-pane digit shortcuts live on the popover, not the list: after a
          // model pick focus sits on that row in the other pane, and the key
          // has to work from wherever focus is — except while typing a search.
          if (!twoPane || !showEffort || effortDisabled || !onEffortChange || !/^[1-9]$/.test(event.key)) return;
          if (event.target instanceof HTMLInputElement) return;
          const level = effortLevels[Number(event.key) - 1];
          if (level) { event.preventDefault(); changeEffort(level.value); }
        }}
      >
        {/* Search header: magnifier + input, with a faint keyboard hint at the right. */}
        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
          <Search size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <input
            type="text"
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder="Search models"
            aria-label="Search models"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          {onRefresh && <button type="button" disabled={refreshing} aria-label="Refresh model catalogues" title="Refresh model catalogues" onClick={() => {
            setRefreshing(true);
            setRefreshError(null);
            Promise.resolve().then(onRefresh).catch(() => setRefreshError("Could not refresh models. Try again.")).finally(() => setRefreshing(false));
          }} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50">
            <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" />
          </button>}
        </div>
        {refreshError && <p role="alert" className="px-3 py-2 text-xs text-destructive">{refreshError}</p>}
        <div className={cn("flex min-h-0 flex-1 @max-[410px]/model-picker:flex-col", !twoPane && "flex-col", fit && fit.maxHeight < MIN_POPOVER_HEIGHT && "min-h-24 shrink-0")}>
        <div role="listbox" aria-label={`${roleLabel} models`} aria-busy={switchInFlight || undefined} className={cn("min-h-0 flex-1 overflow-y-auto p-1.5", twoPane && "w-[210px] flex-none border-r border-border @max-[410px]/model-picker:max-h-44 @max-[410px]/model-picker:w-full @max-[410px]/model-picker:border-r-0 @max-[410px]/model-picker:border-b", switchInFlight && "cursor-progress")}>
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
                  <span className={cn("text-[11px] font-medium uppercase tracking-[0.12em]", adapter.available ? "text-muted-foreground" : "text-muted-foreground")}>{adapter.label}</span>
                  {adapter.capabilities.includes("reasoning") && <span className="rounded-full border border-border px-1.5 py-0.5 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">thinking</span>}
                  {!adapter.available && <span className="ml-auto text-[11px] text-muted-foreground">unavailable</span>}
                </div>
                {models.map(option => {
                  const selected = adapter.id === activeHarness && (option.id ? option.id === activeModel : !activeModel);
                  const selectable = adapter.available && option.available !== false && option.compatible !== false;
                  return <button key={`${adapter.id}:${option.id || "default"}`} type="button" role="option" aria-selected={selected} disabled={!selectable} onClick={() => pickModel(adapter.id as Harness, option.id || null)} className={cn("flex h-9 w-full items-center gap-2 rounded-[7px] px-2 text-left transition-colors disabled:opacity-40", selected ? "bg-accent" : "hover:bg-accent", !selectable && "opacity-60")}>
                    <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[13px] text-foreground">{cleanModelLabel(option.label)}</span>
                    {option.lifecycle === "preview" && <span className="rounded-full border border-border px-1.5 py-0.5 font-mono text-[11px] uppercase text-muted-foreground">preview</span>}
                    {selected && <Check size={13} className="shrink-0 text-foreground" aria-hidden="true" />}
                  </button>;
                })}
              </div>
            ))}
        </div>
        {twoPane && <div className="flex min-w-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3">
          <div className="flex items-center gap-2 text-[13px] font-semibold text-foreground">
            <HarnessMark harness={activeHarness} size={12} />
            <span className="min-w-0 flex-1 truncate">{modelLabel}</span>
            {showEffort && <span className="rounded-full border border-border px-1.5 py-0.5 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">thinking</span>}
          </div>
          <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">Thinking effort</div>
          {showEffort
            ? <div data-testid="effort-control"><EffortList {...effortProps} /></div>
            : <p className="text-[12px] leading-5 text-muted-foreground">No thinking control for this model.</p>}
          {showEffort && <p className="mt-auto pt-1 text-[11px] text-faint">1–{Math.min(9, effortLevels.length)} jumps to a level</p>}
        </div>}
        </div>
        {!twoPane && showEffort && (
          <div className="h-[72px] shrink-0 border-t border-border px-3.5 pb-2 pt-2.5 [contain:layout]" data-testid="effort-control">
            {effortStyle === "sentence" ? <EffortSentence {...effortProps} /> : <EffortSlider {...effortProps} />}
          </div>
        )}
        <div className="shrink-0 border-t border-border px-3 py-2.5">
          <p className="text-[12px] leading-4 text-faint">Switching restarts the provider session. History stays.</p>
        </div>
      </div>
    </>}
  </div>;
}
