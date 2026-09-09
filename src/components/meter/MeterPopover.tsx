// The menu-bar meter popover: CodexBar's menu card (`docs/ui.md`) rendered in
// Bridge's chrome. A ring gauge per provider for the window closest to
// biting, then one bar per quota window with its reset and pace in words.
// Pace math is the shared `src/meter.ts` port; styling is Tailwind utilities
// plus `.u-glass-popover` only. Series colour follows the harness.
import { useEffect, useRef } from "react";
import { Gauge, RefreshCw, X } from "lucide-react";
import type { UsageProvider, UsageSnapshot } from "../../usage";
import type { MeterRegistry } from "../../types";
import { HarnessMark } from "../harnessMarks";
import { MeterReadings, providerSnapshots, useMeterClock } from "./MeterReadings";

export function MeterPopover({ usage, registry, refreshing, onRefresh, onClose, onOpenBridge }: {
  usage: Partial<Record<UsageProvider, UsageSnapshot>>;
  registry: MeterRegistry | null;
  refreshing: boolean;
  onRefresh: () => void;
  onClose: () => void;
  /** Present only on the menu-bar panel, which is otherwise a dead end: from
   *  the menu bar there is no other way through to the app. */
  onOpenBridge?: () => void;
}) {
  const nowMs = useMeterClock();
  const dialogRef = useRef<HTMLDivElement>(null);
  // A dialog that opens without focus strands keyboard users; focus the
  // dialog itself on mount (Escape is handled globally, topmost-layer-first).
  useEffect(() => { dialogRef.current?.focus(); }, []);
  const live = providerSnapshots(usage);
  const liveProviders = new Set(live.map(entry => entry.provider));
  const awaiting = (registry?.providers ?? []).filter(entry => entry.supported && !liveProviders.has(entry.id as UsageProvider));
  const worst = live.flatMap(({ snapshot }) => snapshot.windows).reduce<number | null>((max, window) => (max == null ? window.usedPercent : Math.max(max, window.usedPercent)), null);
  // The meter is its own menu-bar window, so the card fills that window rather
  // than floating inside the app. Nothing sits behind it to be made inert,
  // which is why there is no `aria-modal` here.
  return <div ref={dialogRef} role="dialog" aria-label="Usage meter" tabIndex={-1} className="u-glass-popover flex h-dvh w-full flex-col overflow-hidden rounded-2xl outline-none">
    <div className="flex items-center gap-2 border-b border-border px-3.5 py-2.5">
      <Gauge size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <h2 className="font-display text-sm font-semibold text-foreground">Meter</h2>
      {/* The worst window is the headline — it is the one about to bite. Kept
          terse because the header has to survive a 360pt panel. */}
      <span className="truncate text-caption tabular-nums text-muted-foreground">{worst == null ? "no limits" : `${Math.round(worst)}% worst`}</span>
      <span className="ml-auto flex shrink-0 items-center gap-1">
        {onOpenBridge && <button type="button" onClick={onOpenBridge} className="rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Open Bridge</button>}
        <button type="button" onClick={onRefresh} disabled={refreshing} aria-label="Refresh meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
          <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" />
        </button>
        <button type="button" onClick={onClose} aria-label="Close meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
          <X size={13} aria-hidden="true" />
        </button>
      </span>
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto px-3.5 py-2">
      {registry == null && live.length === 0 && <div role="status" className="flex items-center justify-center gap-2 py-5 text-caption text-muted-foreground"><RefreshCw size={12} className="animate-spin" aria-hidden="true" />Loading meter</div>}
      <MeterReadings usage={usage} nowMs={nowMs} />
      {awaiting.map(entry => <section key={entry.id} aria-label={`${entry.label} usage`} className="flex items-center gap-2 border-b border-border py-3 last:border-0">
        <HarnessMark harness={entry.id} size={12} />
        <span className="font-medium text-foreground">{entry.label}</span>
        <span className="ml-auto text-[11px] text-muted-foreground">Awaiting live usage</span>
      </section>)}
    </div>
  </div>;
}
