import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { UsageOverviewSnapshot, UsageResetCredit } from "../protocol/generated/protocol";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { harnessLabel } from "../utils";

function dateLabel(seconds: number): string {
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(new Date(seconds * 1000));
}

export function resetSummary(snapshot: UsageOverviewSnapshot): string | null {
  const bank = snapshot.resetCredits;
  if (!bank || bank.availableCount === 0 || (bank.availableCount == null && bank.credits.length === 0)) return null;
  const count = bank.availableCount == null ? "Unknown resets banked" : `${bank.availableCount} ${bank.availableCount === 1 ? "reset" : "resets"} banked`;
  return bank.nextExpiresAt ? `${count} · earliest expires ${dateLabel(bank.nextExpiresAt)}` : count;
}

function pickCredit(snapshot: UsageOverviewSnapshot): { credit: UsageResetCredit | undefined; reason: string | null } {
  const bank = snapshot.resetCredits;
  if (!bank) return { credit: undefined, reason: "No reset available" };
  const credit = bank.credits.find(item => item.usableNow !== false);
  if (credit) return { credit, reason: null };
  if ((bank.availableCount ?? 0) > 0 && snapshot.provider === "codex") return { credit: undefined, reason: null };
  const first = bank.credits[0];
  return { credit: first, reason: first?.requiresLimit ? "Available when you reach a limit" : "Reset is not usable yet" };
}

function confirmation(snapshot: UsageOverviewSnapshot, credit: UsageResetCredit | undefined): string[] {
  const clears = credit?.clears ?? (snapshot.provider === "codex" ? ["session", "weekly"] : []);
  const weekly = clears.includes("weekly") || clears.includes("seven_day");
  const lines = [weekly
    ? `Refills your 5-hour and weekly limits. Your weekly reset date moves to about ${dateLabel(Math.floor(Date.now() / 1000) + 7 * 86400)}.`
    : "Refills your session limit. Your weekly reset day stays the same."];
  const used = snapshot.windows.find(window => window.id === "weekly")?.usedPercent.value;
  if (weekly && used != null && used < 100) lines.push(`You still have ${Math.round(100 - used)}% of your weekly limit left. Use your reset anyway?`);
  return lines;
}

const OUTCOMES: Record<string, string> = {
  reset: "Reset used. Usage limits are refreshing.",
  nothingToReset: "Your usage does not need a reset right now.",
  noCredit: "No reset credit is available.",
  alreadyRedeemed: "This reset was already used.",
  cooldown: "This reset is cooling down.",
  ineligible: "This account is not eligible for this reset.",
  unavailable: "This reset is currently unavailable.",
  unconfirmed: "The result is unconfirmed. Usage was refreshed; retry will reuse the same request.",
  authError: "Reconnect this provider before using a reset.",
};

export function UsageResetRow({ snapshot, onUpdated, compact = false }: { snapshot: UsageOverviewSnapshot; onUpdated?: () => void; compact?: boolean }) {
  const summary = resetSummary(snapshot);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const pending = useRef<{ key: string; creditId: string | null } | null>(null);
  if (!summary) return result ? <p role="status" className="text-caption text-muted-foreground">{result}</p> : null;
  const { credit, reason } = pickCredit(snapshot);
  const age = snapshot.observedAt == null ? Infinity : Date.now() / 1000 - snapshot.observedAt;
  const disabledReason = reason ?? (!snapshot.account ? "Sign in to use a reset" : snapshot.error || age < 0 || age >= 600 ? "Refresh account usage before using a reset" : null);
  const disabled = busy || !!disabledReason;
  const redeem = async () => {
    if (busy) return;
    setBusy(true);
    setConfirming(false);
    pending.current ??= { key: crypto.randomUUID(), creditId: credit?.id ?? null };
    try {
      const response = await bridgeApi.redeemProviderUsageReset({ provider: snapshot.provider, creditId: pending.current.creditId, idempotencyKey: pending.current.key });
      setResult(OUTCOMES[response.outcome] ?? "Reset request completed.");
      if (response.outcome !== "unconfirmed") pending.current = null;
      onUpdated?.();
    } catch (error) {
      setResult(errorMessage(error));
    } finally { setBusy(false); }
  };
  return <div className={compact ? "mt-2 border-t border-border pt-2 text-[11px]" : "u-glass-soft rounded-xl p-3 text-caption"}>
    <div className="flex items-center justify-between gap-3">
      <span className="text-foreground">{compact ? summary : `${harnessLabel(snapshot.provider)} · ${summary}`}</span>
      <button type="button" onClick={() => setConfirming(true)} disabled={disabled} title={disabledReason ?? undefined} className="shrink-0 rounded-md border border-border px-2.5 py-1 font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-40">Use reset</button>
    </div>
    {disabledReason && <p className="mt-1 text-muted-foreground">{disabledReason}</p>}
    {result && <p role="status" className="mt-1 text-muted-foreground">{result}</p>}
    {confirming && createPortal(<div className="fixed inset-0 z-[100] grid place-items-center bg-background/70 p-4" onPointerDown={event => event.stopPropagation()}>
      <div role="dialog" aria-modal="true" aria-label={`Use ${harnessLabel(snapshot.provider)} reset`} className="u-glass-popover w-full max-w-sm rounded-2xl border border-border p-5 shadow-xl">
        <h2 className="font-display text-base font-semibold text-foreground">Use {harnessLabel(snapshot.provider)} reset?</h2>
        {confirmation(snapshot, credit).map(line => <p key={line} className="mt-2 text-ui leading-relaxed text-muted-foreground">{line}</p>)}
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={() => setConfirming(false)} className="rounded-lg border border-border px-3 py-1.5 text-ui text-foreground hover:bg-accent">Cancel</button>
          <button type="button" onClick={() => void redeem()} className="rounded-lg bg-primary px-3 py-1.5 text-ui font-medium text-primary-foreground hover:bg-primary/90">Use reset</button>
        </div>
      </div>
    </div>, document.body)}
  </div>;
}
