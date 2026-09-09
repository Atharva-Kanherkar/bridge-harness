// The menu-bar meter popover: CodexBar's menu card (`docs/ui.md`) rendered in
// Bridge's chrome. One tile per live provider window — usage bar, reset
// countdown, and the pace line ("12% in deficit · runs out in 3h") — plus the
// planned-provider matrix from the registry so follow-ups are visible, not
// silent. Pace math is the shared `src/meter.ts` port; styling is Tailwind
// utilities plus `.u-glass-popover` only.
import { useMemo, useState } from "react";
import { Gauge, RefreshCw, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatReset, windowLabel, type RateWindow, type UsageProvider, type UsageSnapshot } from "../../usage";
import { paceLabel, paceTokenDelta, paceVisible, paceWeekly } from "../../meter";
import type { AdapterDescriptor, MeterRegistry } from "../../types";
import { HarnessMark } from "../harnessMarks";
import { harnessLabel } from "../../utils";

const PROVIDER_ORDER: UsageProvider[] = ["codex", "claude", "cursor", "opencode"];

function providerSnapshots(usage: Partial<Record<UsageProvider, UsageSnapshot>>): Array<{ provider: UsageProvider; snapshot: UsageSnapshot }> {
  return PROVIDER_ORDER.filter(provider => usage[provider]).map(provider => ({ provider, snapshot: usage[provider]! }));
}

function WindowRow({ window, nowMs }: { window: RateWindow; nowMs: number }) {
  const pace = useMemo(() => paceWeekly(window, nowMs), [window, nowMs]);
  const showPace = pace != null && paceVisible(window, nowMs);
  const reset = window.resetsInSeconds != null ? formatReset(window.resetsInSeconds) : window.resetsLabel;
  const used = Math.min(100, Math.max(0, window.usedPercent));
  return <div className="py-1.5">
    <div className="flex items-baseline justify-between gap-2 text-caption">
      <span className="font-medium text-foreground">{window.label || windowLabel(window.id, window.windowMinutes)}</span>
      <span className="shrink-0 tabular-nums text-muted-foreground">{Math.round(used)}% used{pace && showPace ? ` · ${paceTokenDelta(pace)}` : ""}</span>
    </div>
    <span className="mt-1 block h-1.5 w-full overflow-hidden rounded-full bg-muted" role="img" aria-label={`${window.label} ${Math.round(used)} percent used`}>
      <span className="block h-full rounded-full bg-foreground" style={{ width: `${used}%` }} />
    </span>
    <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
      {reset && <span className="tabular-nums">{reset}</span>}
      {pace && showPace && <span>{paceLabel(pace)}</span>}
    </div>
  </div>;
}

export function MeterPopover({ usage, registry, refreshing, onRefresh, onClose }: {
  usage: Partial<Record<UsageProvider, UsageSnapshot>>;
  adapters?: AdapterDescriptor[];
  registry: MeterRegistry | null;
  refreshing: boolean;
  onRefresh: () => void;
  onClose: () => void;
}) {
  const [nowMs] = useState(() => Date.now());
  const live = providerSnapshots(usage);
  const planned = (registry?.providers ?? []).filter(entry => !entry.supported);
  const worst = live.flatMap(({ snapshot }) => snapshot.windows).reduce<number | null>((max, window) => (max == null ? window.usedPercent : Math.max(max, window.usedPercent)), null);
  return <div role="dialog" aria-label="Usage meter" className="u-glass-popover flex max-h-[80dvh] w-[min(24rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl">
    <div className="flex items-center gap-2 border-b border-border px-3.5 py-2.5">
      <Gauge size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <h2 className="font-display text-sm font-semibold text-foreground">Meter</h2>
      <span className="truncate text-caption tabular-nums text-muted-foreground">{worst == null ? "no live windows" : `${Math.round(worst)}% worst window`}</span>
      <span className="ml-auto flex items-center gap-1">
        <button type="button" onClick={onRefresh} disabled={refreshing} aria-label="Refresh meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
          <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" />
        </button>
        <button type="button" onClick={onClose} aria-label="Close meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
          <X size={13} aria-hidden="true" />
        </button>
      </span>
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto px-3.5 py-2">
      {live.length === 0 && <p className="py-4 text-center text-caption text-muted-foreground">No live windows yet. Run a session, then refresh.</p>}
      {live.map(({ provider, snapshot }) => <section key={provider} aria-label={`${harnessLabel(provider)} usage`} className="border-b border-border py-1.5 last:border-0">
        <div className="flex items-center gap-1.5 text-ui">
          <HarnessMark harness={provider} size={12} />
          <span className="font-medium text-foreground">{harnessLabel(provider)}</span>
          {snapshot.planType && <span className="text-[11px] text-muted-foreground">{snapshot.planType}</span>}
        </div>
        {snapshot.windows.length === 0 && <p className="py-1 text-[11px] text-muted-foreground">Connected — no quota windows reported.</p>}
        {snapshot.windows.map(window => <WindowRow key={window.id} window={window} nowMs={nowMs} />)}
      </section>)}
      {planned.length > 0 && <details className="py-2 text-caption text-muted-foreground">
        <summary className="cursor-pointer hover:text-foreground">{planned.length} more providers planned</summary>
        <ul className="mt-1.5 space-y-1">
          {planned.map(entry => <li key={entry.id} className="flex items-baseline justify-between gap-2">
            <span className={cn("text-foreground")}>{entry.label}</span>
            <span className="truncate text-[11px]">{entry.plannedSource}</span>
          </li>)}
        </ul>
      </details>}
    </div>
    <p className="border-t border-border px-3.5 py-2 text-[11px] text-muted-foreground">Pace math from CodexBar (MIT). Limits refresh from your provider sessions — no passwords stored.</p>
  </div>;
}
