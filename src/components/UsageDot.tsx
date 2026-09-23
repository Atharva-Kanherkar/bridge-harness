// The chat's usage dot: a ring beside the composer coloured by the tightest
// live quota window across every provider, opening a card that reads exactly
// like the native menu-bar meter. Both surfaces consume the same versioned
// `ProviderUsageOverviews` snapshot, so a number here is the number up there.
// Quota semantics (real zero, stale, unavailable) come from the backend; this
// file draws them and never infers a value.
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { BarChart3, RefreshCw, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import type { ProviderUsageOverviews, UsageOverviewSnapshot, UsageQuotaWindow } from "../protocol/generated/protocol";
import type { AdapterDescriptor } from "../types";
import { formatReset, type UsageProvider } from "../usage";
import { HarnessMark, harnessChartDot } from "./harnessMarks";
import { harnessLabel } from "../utils";
import { useMeterClock } from "./meter/MeterReadings";
import { UsageResetRow } from "./UsageResetRow";

export const PROVIDER_ORDER: UsageProvider[] = ["codex", "claude", "cursor", "opencode"];

/** An observation older than this is history, not a live limit — the same
 *  ten-minute horizon the menu bar and `overviewUsage` apply. */
const FRESH_SECONDS = 600;

export type UsageTier = "unknown" | "ok" | "warning" | "critical";

export function usageTier(percent: number | null): UsageTier {
  if (percent == null) return "unknown";
  if (percent >= 90) return "critical";
  if (percent >= 70) return "warning";
  return "ok";
}

const TIER_RING_CLASS: Record<UsageTier, string> = {
  unknown: "text-muted-foreground",
  ok: "text-success",
  warning: "text-warning",
  critical: "text-destructive",
};

/** What the ring's colour means, in words, so the accessible name never
 *  depends on hue alone. */
const TIER_LABEL: Record<UsageTier, string> = {
  unknown: "unknown",
  ok: "healthy",
  warning: "elevated",
  critical: "critical",
};

const clamp = (value: number) => Math.min(100, Math.max(0, value));

/** A provider's snapshot is live when it read successfully within the fresh
 *  horizon; an error or an old observation makes every window historical. */
export function snapshotFresh(snapshot: UsageOverviewSnapshot, nowSeconds: number): boolean {
  if (snapshot.error) return false;
  if (snapshot.observedAt == null) return false;
  return nowSeconds >= snapshot.observedAt && nowSeconds - snapshot.observedAt < FRESH_SECONDS;
}

/** The current percentage of a window, or null when it is stale, unavailable,
 *  or has already reset. A reset proves expiry, never a fresh zero. */
export function liveUsedPercent(window: UsageQuotaWindow, nowSeconds: number): number | null {
  if (window.usedPercent.status !== "current" || window.usedPercent.value == null) return null;
  if (window.resetsAt != null && window.resetsAt <= nowSeconds) return null;
  return clamp(window.usedPercent.value);
}

/** The single number the ring colours itself by: the highest live usage of
 *  any window on any provider. Providers without a live reading stay out of
 *  the computation rather than counting as 0% used. */
export function worstLiveUsage(overviews: ProviderUsageOverviews | null, nowSeconds: number): { provider: UsageProvider; window: UsageQuotaWindow; used: number } | null {
  let worst: { provider: UsageProvider; window: UsageQuotaWindow; used: number } | null = null;
  for (const snapshot of overviews?.providers ?? []) {
    if (!snapshotFresh(snapshot, nowSeconds)) continue;
    for (const window of snapshot.windows) {
      const used = liveUsedPercent(window, nowSeconds);
      if (used == null) continue;
      if (worst == null || used > worst.used) worst = { provider: snapshot.provider as UsageProvider, window, used };
    }
  }
  return worst;
}

export function orderedSnapshots(overviews: ProviderUsageOverviews | null): UsageOverviewSnapshot[] {
  const byProvider = new Map((overviews?.providers ?? []).map(snapshot => [snapshot.provider, snapshot]));
  return PROVIDER_ORDER.flatMap(provider => {
    const snapshot = byProvider.get(provider);
    return snapshot ? [snapshot] : [];
  });
}

/** The compact trigger: one ring sized to sit beside the composer's send
 *  button. Track always; arc only when there is a live reading to draw. */
export function UsageRing({ percent, tier }: { percent: number | null; tier: UsageTier }) {
  const radius = 8;
  const circumference = 2 * Math.PI * radius;
  const drawn = percent == null ? 0 : clamp(percent);
  return <svg width="20" height="20" viewBox="0 0 20 20" className="shrink-0 -rotate-90" aria-hidden="true">
    <circle cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" stroke="currentColor" className="text-foreground/10" />
    {percent != null && <circle cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" strokeLinecap="round" stroke="currentColor" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - drawn / 100)} className={cn(TIER_RING_CLASS[tier], "transition-[stroke-dashoffset,color] duration-700 ease-out motion-reduce:transition-none")} />}
  </svg>;
}

function percentLabel(value: number): string {
  if (value > 0 && value < 0.01) return "<0.01%";
  if (value > 99.99 && value < 100) return ">99.99%";
  if ((value > 0 && value < 1) || (value > 99 && value < 100)) return `${value.toFixed(2)}%`;
  return `${Math.round(value)}%`;
}

/** The menu bar's bar, in Tailwind: a rounded track, the provider's fill, and
 *  fixed guide marks at 50 and 75 percent punched through the fill so they
 *  read at any value. Null fill means unavailable, never an observed zero. */
export function QuotaBar({ provider, fill }: { provider: UsageProvider; fill: number | null }) {
  return <div className="relative mt-1 h-1.5 w-full overflow-hidden rounded-full bg-muted" aria-hidden="true">
    {fill != null && fill > 0 && <span className={cn("absolute inset-y-0 left-0 rounded-full motion-safe:animate-[meter-fill_600ms_ease-out] origin-left", harnessChartDot(provider))} style={{ width: `${clamp(fill)}%` }} />}
    {[50, 75].map(mark => <span key={mark} data-quota-marker={mark} className="absolute inset-y-0 w-[5px] -translate-x-1/2 bg-popover" style={{ left: `${mark}%` }}>
      <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-foreground/60" />
    </span>)}
  </div>;
}

function qualifier(window: UsageQuotaWindow, fresh: boolean, nowSeconds: number): string | null {
  const { status, source, value } = window.usedPercent;
  if (status === "unavailable" || value == null) return "Unavailable";
  if (status === "stale" || !fresh) return "Stale";
  if (window.resetsAt != null && window.resetsAt <= nowSeconds) return "Stale";
  return source === "estimated" ? "Estimated" : null;
}

function WindowRow({ provider, window, fresh, nowSeconds }: { provider: UsageProvider; window: UsageQuotaWindow; fresh: boolean; nowSeconds: number }) {
  const live = fresh ? liveUsedPercent(window, nowSeconds) : null;
  const recorded = window.usedPercent.value != null && window.usedPercent.status !== "unavailable" ? clamp(window.usedPercent.value) : null;
  const note = qualifier(window, fresh, nowSeconds);
  const reset = window.resetsAt != null ? formatReset(window.resetsAt - nowSeconds) : undefined;
  return <div className="py-1.5" data-quota-window={window.id}>
    <div className="flex items-baseline justify-between gap-2 text-caption">
      <span className="truncate font-medium text-foreground">{window.label}</span>
      <span className="shrink-0 tabular-nums text-muted-foreground">
        {live != null ? <>{percentLabel(live)} used <span className="text-muted-foreground/60">· {percentLabel(100 - live)} left</span></>
          : recorded != null ? <>{percentLabel(recorded)} used <span className="text-muted-foreground/60">· {note}</span></>
          : note}
      </span>
    </div>
    <QuotaBar provider={provider} fill={live ?? recorded} />
    {(reset || (live != null && note)) && <div className="mt-1 flex flex-wrap gap-x-3 text-[11px] text-muted-foreground">
      {reset && <span className="tabular-nums">{reset}</span>}
      {live != null && note && <span>{note}</span>}
    </div>}
  </div>;
}

/** A named auth state wins over install status: an adapter can only report
 *  signed_out about a CLI it found, so a signed-out Cursor is unavailable and
 *  signed_out at once, and Sign in is the useful route. */
function adapterState(adapter?: AdapterDescriptor): "not_installed" | "signed_out" | "normal" {
  if (!adapter) return "normal";
  if (adapter.authState === "signed_out") return "signed_out";
  if (!adapter.available) return "not_installed";
  return "normal";
}

function ProviderSection({ snapshot, adapter, nowSeconds, onSignIn, onRefresh }: { snapshot: UsageOverviewSnapshot; adapter?: AdapterDescriptor; nowSeconds: number; onSignIn?: (provider: UsageProvider) => void; onRefresh?: () => void }) {
  const provider = snapshot.provider as UsageProvider;
  const fresh = snapshotFresh(snapshot, nowSeconds);
  const state = adapterState(adapter);
  const meta = [snapshot.account, snapshot.plan].filter((value): value is string => !!value).join(" · ");
  return <section aria-label={`${harnessLabel(provider)} usage`} className="border-b border-border py-2.5 last:border-0">
    <div className="flex min-w-0 items-center gap-1.5 text-ui">
      <HarnessMark harness={provider} size={12} />
      <span className="font-medium text-foreground">{harnessLabel(provider)}</span>
      <span className="ml-auto min-w-0 truncate text-[11px] text-muted-foreground">
        {state === "not_installed" ? "Not installed"
          : state === "signed_out" ? <button type="button" onClick={() => onSignIn?.(provider)} className="min-h-6 rounded-md border border-border px-2 text-[11px] font-medium text-foreground transition-colors hover:bg-accent">Sign in</button>
          : meta}
      </span>
    </div>
    {snapshot.windows.length
      ? snapshot.windows.map(window => <WindowRow key={window.id} provider={provider} window={window} fresh={fresh} nowSeconds={nowSeconds} />)
      : <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{snapshot.error ?? (state === "not_installed" ? `${harnessLabel(provider)} isn't installed. Add it in Settings → Harnesses.` : "No quota reported yet.")}</p>}
    {snapshot.windows.length > 0 && snapshot.error && <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{snapshot.error}</p>}
    <UsageResetRow snapshot={snapshot} onUpdated={onRefresh} compact />
  </section>;
}

export interface UsageDotProps {
  overviews: ProviderUsageOverviews | null;
  adapters?: AdapterDescriptor[];
  refreshing?: boolean;
  onRefresh?: () => void;
  onOpenUsage?: () => void;
  onSignIn?: (provider: UsageProvider) => void;
  /** Rides the composer: the card portals onto `[data-composer-frame]` and
   *  hangs above it. Otherwise it drops below the trigger. */
  compact?: boolean;
  /** The last load or refresh failure, shown in the card until a snapshot
   *  arrives. Without it a failed first read would spin forever. */
  error?: string | null;
  /** Test seam; production reads the ticking meter clock. */
  nowMs?: number;
}

export const UsageDot = memo(function UsageDot({ overviews, adapters, refreshing = false, onRefresh, onOpenUsage, onSignIn, compact = false, error = null, nowMs }: UsageDotProps) {
  const [open, setOpen] = useState(false);
  const clock = useMeterClock();
  // The timer ages idle readings. A newly pushed snapshot can arrive between
  // ticks, so judge it against the current wall clock rather than the old tick.
  const now = (nowMs ?? Math.max(clock, Date.now())) / 1000;
  const [frame, setFrame] = useState<HTMLElement | null>(null);
  const [availableHeight, setAvailableHeight] = useState<number>();
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  // Compact rides the composer, so its card belongs to the pill's own box:
  // portalling onto `[data-composer-frame]` anchors it to the composer's edge
  // rather than to a button that scrolls with the controls row.
  useEffect(() => {
    if (!compact) { setFrame(null); return; }
    setFrame(rootRef.current?.closest<HTMLElement>("[data-composer-frame]") ?? null);
  }, [compact]);

  // A tall composer leaves less room than a viewport fraction allows; keep the
  // card below the toolbar and above its anchor.
  useEffect(() => {
    if (!compact || !open) return;
    const anchor = frame ?? rootRef.current;
    if (!anchor) return;
    const main = anchor.closest("main");
    const measure = () => {
      let top = Math.max(0, main?.getBoundingClientRect().top ?? 0);
      for (let parent = anchor.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent);
        if (/(auto|scroll|hidden|clip)/.test(style.overflowY || style.overflow)) top = Math.max(top, parent.getBoundingClientRect().top);
      }
      setAvailableHeight(Math.max(0, Math.min(window.innerHeight * 0.8, anchor.getBoundingClientRect().top - top - 12)));
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(anchor);
    if (main) observer?.observe(main);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [compact, frame, open]);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      setOpen(false);
    };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", dismiss); document.removeEventListener("keydown", escape); };
  }, [open]);

  const worst = worstLiveUsage(overviews, now);
  const tier = usageTier(worst?.used ?? null);
  const stateLabel = worst == null ? "no live usage yet" : `${TIER_LABEL[tier]}, ${percentLabel(worst.used)} of ${harnessLabel(worst.provider)} ${worst.window.label} used`;
  const snapshots = orderedSnapshots(overviews);
  const adapterFor = (provider: string) => adapters?.find(item => item.id === provider);

  const panel = (
    <div
      ref={panelRef}
      id="usage-dot-panel"
      role="dialog"
      aria-label="Usage"
      className={cn(
        "absolute z-50",
        // The same 360pt card as the menu-bar meter, hung from the composer's
        // right edge and just clear of it: its own shape, never a seam.
        compact ? "bottom-full right-0 mb-1.5 w-[min(100vw-1.5rem,22.5rem)]" : "right-0 top-full pt-2 w-[min(100vw-1.5rem,22.5rem)]",
        "transition-[opacity,transform] duration-200 ease-out motion-reduce:transition-none",
        open ? "visible translate-y-0 pointer-events-auto opacity-100" : "invisible translate-y-1 pointer-events-none opacity-0",
      )}
    >
      <div style={compact && availableHeight !== undefined ? { maxHeight: availableHeight } : undefined} className="u-glass-popover flex max-h-[70dvh] w-full flex-col overflow-hidden rounded-2xl">
        <div className="flex items-center gap-2 border-b border-border px-3.5 py-2.5">
          <h2 className="font-display text-sm font-semibold text-foreground">Usage</h2>
          <span className="truncate text-caption tabular-nums text-muted-foreground">{worst == null ? "no live limits" : `${percentLabel(worst.used)} · ${harnessLabel(worst.provider)} ${worst.window.label}`}</span>
          <span className="ml-auto flex shrink-0 items-center gap-1">
            {onOpenUsage && <button type="button" onClick={() => { setOpen(false); onOpenUsage(); }} className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><BarChart3 size={11} aria-hidden="true" />Open Usage</button>}
            {onRefresh && <button type="button" onClick={onRefresh} disabled={refreshing} aria-label="Refresh usage" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"><RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" /></button>}
            <button type="button" onClick={() => setOpen(false)} aria-label="Close usage" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><X size={13} aria-hidden="true" /></button>
          </span>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-3.5 py-1">
          {error && <p role="alert" className="my-2 rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-2 text-[11px] leading-relaxed text-destructive">{error}</p>}
          {overviews == null && !error && <div role="status" className="flex items-center justify-center gap-2 py-5 text-caption text-muted-foreground"><RefreshCw size={12} className="animate-spin" aria-hidden="true" />Loading usage</div>}
          {snapshots.map(snapshot => <ProviderSection key={snapshot.provider} snapshot={snapshot} adapter={adapterFor(snapshot.provider)} nowSeconds={now} onSignIn={onSignIn} onRefresh={onRefresh} />)}
          {overviews != null && snapshots.length === 0 && <p className="py-5 text-center text-caption text-muted-foreground">No providers report usage yet.</p>}
        </div>
      </div>
    </div>
  );

  return <div ref={rootRef} className="relative">
    <button
      type="button"
      className={cn(
        compact
          ? "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors duration-150 hover:bg-accent hover:text-foreground active:scale-95"
          : "relative grid size-8 shrink-0 cursor-pointer place-items-center rounded-full border border-border bg-card transition-colors hover:bg-accent",
        open && "bg-accent",
      )}
      aria-label={`${open ? "Close" : "Open"} usage — ${stateLabel}`}
      aria-expanded={open}
      aria-controls="usage-dot-panel"
      title={`Usage — ${stateLabel}`}
      onClick={() => setOpen(value => !value)}
    >
      <UsageRing percent={worst?.used ?? null} tier={tier} />
    </button>
    {compact && frame ? createPortal(panel, frame) : panel}
  </div>;
});

/** Subscribes to the shared provider overviews the menu bar renders, so the
 *  chat dot and the status item never disagree. One subscription per mounted
 *  dot; only one composer is mounted at a time. */
export function useProviderUsageOverviews(): { overviews: ProviderUsageOverviews | null; refreshing: boolean; refresh: () => void; error: string | null } {
  const [overviews, setOverviews] = useState<ProviderUsageOverviews | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Every source — initial read, pushed event, interactive refresh — passes
  // through one watermark, so a late event generated before a refresh can
  // never roll the dot back to older quota data.
  const latest = useRef(-Infinity);
  const active = useRef(true);
  const accept = useCallback((value: ProviderUsageOverviews | null) => {
    if (!active.current || !value || value.generatedAt < latest.current) return;
    latest.current = value.generatedAt;
    setOverviews(value);
    setError(null);
  }, []);
  useEffect(() => {
    active.current = true;
    let off: (() => void) | undefined;
    void bridgeApi.getProviderUsageOverviews().then(accept).catch(value => { if (active.current) setError(errorMessage(value)); });
    void bridgeApi.onProviderUsageOverviews(accept).then(fn => { if (active.current) off = fn; else fn(); });
    return () => { active.current = false; off?.(); };
  }, [accept]);
  const refresh = useCallback(() => {
    setRefreshing(true);
    bridgeApi.refreshProviderUsageOverviews()
      .then(accept)
      .catch(value => { if (active.current) setError(errorMessage(value)); })
      .finally(() => { if (active.current) setRefreshing(false); });
  }, [accept]);
  return { overviews, refreshing, refresh, error };
}

/** The dot as the composer mounts it: live data, refresh wired, worker and
 *  chat composers alike. */
export function ChatUsageDot({ adapters, onOpenUsage, onSignIn, compact = true }: { adapters?: AdapterDescriptor[]; onOpenUsage?: () => void; onSignIn?: (provider: UsageProvider) => void; compact?: boolean }) {
  const { overviews, refreshing, refresh, error } = useProviderUsageOverviews();
  return <UsageDot overviews={overviews} adapters={adapters} refreshing={refreshing} onRefresh={refresh} onOpenUsage={onOpenUsage} onSignIn={onSignIn} compact={compact} error={error} />;
}
