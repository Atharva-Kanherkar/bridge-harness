// Adapter for the existing in-app presentation. Quota authority and semantics
// come from the versioned backend snapshot; this contains no provider logic.
import type { UsageOverviewSnapshot } from "./protocol/generated/protocol";
import type { UsageSnapshot } from "./usage";

export function overviewUsage(snapshot: UsageOverviewSnapshot): UsageSnapshot | null {
  if (snapshot.schemaVersion !== 1) return null;
  const now = Date.now() / 1000;
  if (snapshot.error || snapshot.observedAt == null || now < snapshot.observedAt || now - snapshot.observedAt >= 600) return null;
  const windows = snapshot.windows.filter(w => w.usedPercent.status === "current" && w.usedPercent.value != null
    && (w.resetsAt == null || w.resetsAt > now)).map(w => ({
    id: w.id, label: w.label, usedPercent: w.usedPercent.value!, windowMinutes: w.windowMinutes ?? undefined,
    resetsInSeconds: w.resetsAt == null ? undefined : Math.max(0, w.resetsAt - now),
    source: w.usedPercent.source ?? "reported" as const,
  }));
  if (!windows.length) return null;
  return { windows, planType: snapshot.plan ?? undefined, source: "reported",
    capturedAt: new Date((snapshot.observedAt ?? snapshot.generatedAt) * 1000).toISOString() };
}
