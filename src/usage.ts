import type { AgentEvent, Harness, Session, SessionStatus, UsageLedgerRow } from "./types";

// Ambient subscription-usage snapshot, parsed from real provider `usage.updated`
// events. Codex (app-server) reports `rate_limits` windows — the same data its
// `/status` command shows — while Claude reports per-turn tokens and cost.
// Everything here is tolerant of naming variants and returns null when the
// provider hasn't reported anything real yet, so we never fabricate numbers.

export interface RateWindow {
  id: string;
  label: string;
  usedPercent: number;
  windowMinutes?: number;
  resetsInSeconds?: number;
  resetsLabel?: string;
  source: MetricSource;
}

export type MetricSource = "reported" | "measured" | "estimated";

export interface UsageSnapshot {
  windows: RateWindow[];
  contextPercent?: number;
  totalTokens?: number;
  costUsd?: number;
  planType?: string;
  model?: string;
  source: MetricSource;
  capturedAt: string;
}

export type UsageProvider = "claude" | "codex" | "opencode";

export interface AccountUsagePayload {
  provider: UsageProvider;
  rateLimits: Record<string, unknown>;
}

type Dict = Record<string, unknown>;

export type ContextPressureLevel = "unknown" | "healthy" | "elevated" | "high" | "critical";

export interface ContextPressure {
  level: ContextPressureLevel;
  label: string;
  explanation: string;
  percent?: number;
}

export interface UsageRateSample {
  usedPercent: number;
  capturedAt: string;
}

export interface UsageProjection {
  projectedAt: string;
  hoursRemaining: number;
  explanation: string;
  source: "estimated";
}

export interface UsageHistoryEntry {
  id: number;
  workUnit: string;
  harness: Harness | "unknown";
  model?: string;
  outcome: SessionStatus | "unknown";
  source: MetricSource;
  totalTokens?: number;
  contextPercent?: number;
  createdAt: string;
}

function isDict(value: unknown): value is Dict {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function num(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "" && Number.isFinite(Number(value))) return Number(value);
  return undefined;
}

export function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value));
}

function pick(source: Dict | undefined, keys: string[]): unknown {
  if (!source) return undefined;
  for (const key of keys) if (key in source) return source[key];
  return undefined;
}

function pickNumber(source: Dict | undefined, keys: string[]): number | undefined {
  return num(pick(source, keys));
}

/** Depth-first search for the provider's rate-limit container. */
function findRateLimits(data: Dict, depth = 0): Dict | undefined {
  const direct = pick(data, ["rate_limits", "rateLimits"]);
  if (isDict(direct)) return direct;
  if (depth > 3) return undefined;
  for (const value of Object.values(data)) {
    if (isDict(value)) {
      const nested = findRateLimits(value, depth + 1);
      if (nested) return nested;
    }
  }
  return undefined;
}

/** Human label for a rolling window given its size in minutes. */
export function windowLabel(id: string, windowMinutes?: number): string {
  if (windowMinutes == null || !Number.isFinite(windowMinutes) || windowMinutes <= 0) {
    return id.charAt(0).toUpperCase() + id.slice(1);
  }
  if (windowMinutes < 60) return `${Math.round(windowMinutes)}m`;
  if (windowMinutes === 1440) return "Daily";
  if (windowMinutes === 10080) return "Weekly";
  if (windowMinutes < 1440) {
    const hours = windowMinutes / 60;
    return `${Number.isInteger(hours) ? hours : hours.toFixed(1)}h`;
  }
  const days = Math.round(windowMinutes / 1440);
  return days === 7 ? "Weekly" : `${days}d`;
}

/** Compact "resets in 2h 5m" style countdown. */
export function formatReset(seconds?: number): string | undefined {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return undefined;
  const total = Math.round(seconds);
  const days = Math.floor(total / 86400);
  const hours = Math.floor((total % 86400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (days > 0) return `resets in ${days}d ${hours}h`;
  if (hours > 0) return `resets in ${hours}h ${minutes}m`;
  if (minutes > 0) return `resets in ${minutes}m`;
  return "resets soon";
}

function windowFrom(id: string, value: Dict, nowMs: number, source: MetricSource): RateWindow | undefined {
  const usedPercent = pickNumber(value, ["used_percent", "usedPercent", "percent_used", "percentUsed", "percent"]);
  if (usedPercent == null) return undefined;
  const windowMinutes = pickNumber(value, ["window_minutes", "windowMinutes", "window_duration_mins", "windowDurationMins", "window_size_minutes", "windowSizeMinutes"]);
  let resetsInSeconds = pickNumber(value, ["resets_in_seconds", "resetsInSeconds", "reset_in_seconds", "seconds_to_reset", "secondsToReset"]);
  if (resetsInSeconds == null) {
    // Codex reports an absolute reset timestamp (`resetsAt`); convert to a countdown.
    const resetsAt = pickNumber(value, ["resets_at", "resetsAt", "reset_at"]);
    if (resetsAt != null) {
      const resetsAtMs = resetsAt > 1e12 ? resetsAt : resetsAt * 1000;
      const delta = (resetsAtMs - nowMs) / 1000;
      if (delta > 0) resetsInSeconds = delta;
    }
  }
  const explicit = pick(value, ["label"]);
  const label = typeof explicit === "string" && explicit.trim() ? explicit : windowLabel(id, windowMinutes);
  const resetsRaw = pick(value, ["resets_label", "resetsLabel"]);
  const resetsLabel = typeof resetsRaw === "string" && resetsRaw.trim() ? resetsRaw : undefined;
  return { id, label, usedPercent, windowMinutes, resetsInSeconds, resetsLabel, source };
}

/** Parse a real usage snapshot from a `usage.updated` event payload. */
export function extractUsageSnapshot(data: unknown): UsageSnapshot | null {
  if (!isDict(data)) return null;
  const usage = isDict(data.usage) ? data.usage : undefined;
  const sourceRaw = pick(data, ["metric_source", "metricSource"]);
  const source: MetricSource = sourceRaw === "measured" || sourceRaw === "estimated" ? sourceRaw : "reported";

  const windows: RateWindow[] = [];
  const rateLimits = findRateLimits(data);
  if (rateLimits) {
    const nowMs = Date.now();
    for (const [id, value] of Object.entries(rateLimits)) {
      if (isDict(value)) {
        const window = windowFrom(id, value, nowMs, source);
        if (window) windows.push(window);
      }
    }
    // Shortest window first (e.g. 5h before Weekly) for a stable, readable order.
    windows.sort((a, b) => (a.windowMinutes ?? Infinity) - (b.windowMinutes ?? Infinity));
  }

  const contextPercent = pickNumber(data, ["context_percent", "contextPercent"]) ?? pickNumber(usage, ["context_percent", "contextPercent"]);
  const input = pickNumber(usage, ["input_tokens", "inputTokens"]);
  const output = pickNumber(usage, ["output_tokens", "outputTokens"]);
  const totalTokens = pickNumber(usage, ["total_tokens", "totalTokens"]) ?? (input != null || output != null ? (input ?? 0) + (output ?? 0) : undefined);
  const costUsd = pickNumber(data, ["total_cost_usd", "totalCostUsd", "cost_usd", "costUsd"]) ?? pickNumber(usage, ["total_cost_usd", "totalCostUsd"]);

  const planRaw = rateLimits ? pick(rateLimits, ["planType", "plan_type", "plan"]) : undefined;
  const planType = typeof planRaw === "string" && planRaw.trim() && planRaw.toLowerCase() !== "unknown" ? planRaw : undefined;
  const modelRaw = pick(data, ["model", "model_id", "modelId"]) ?? pick(usage, ["model", "model_id", "modelId"]);
  const model = typeof modelRaw === "string" && modelRaw.trim() ? modelRaw : undefined;
  const snapshot: UsageSnapshot = { windows, contextPercent, totalTokens, costUsd, planType, model, source, capturedAt: new Date().toISOString() };
  const hasSignal = windows.length > 0 || contextPercent != null || totalTokens != null || costUsd != null;
  return hasSignal ? snapshot : null;
}

/** Explain context health using stable, user-visible thresholds. */
export function contextPressure(percent?: number): ContextPressure {
  if (percent == null || !Number.isFinite(percent)) {
    return { level: "unknown", label: "Context unknown", explanation: "No context measurement has been recorded for this work unit." };
  }
  const clamped = clampPercent(percent);
  if (clamped >= 90) return { level: "critical", label: "Critical pressure", percent: clamped, explanation: "At least 90% of the context window is occupied; compaction or a fresh work unit is recommended." };
  if (clamped >= 75) return { level: "high", label: "High pressure", percent: clamped, explanation: "At least 75% of the context window is occupied; degradation risk is increasing." };
  if (clamped >= 60) return { level: "elevated", label: "Elevated pressure", percent: clamped, explanation: "At least 60% of the context window is occupied; keep the next handoff concise." };
  return { level: "healthy", label: "Healthy context", percent: clamped, explanation: "Context use is below the 60% pressure threshold." };
}

/** Project exhaustion from a real trend without claiming provider precision. */
export function projectUsageExhaustion(samples: UsageRateSample[], horizonHours = 24): UsageProjection | null {
  const ordered = samples
    .filter(sample => Number.isFinite(sample.usedPercent) && Number.isFinite(Date.parse(sample.capturedAt)))
    .map(sample => ({ ...sample, usedPercent: clampPercent(sample.usedPercent) }))
    .sort((a, b) => Date.parse(a.capturedAt) - Date.parse(b.capturedAt));
  if (ordered.length < 3) return null;
  const first = ordered[0];
  const last = ordered[ordered.length - 1];
  const elapsedHours = (Date.parse(last.capturedAt) - Date.parse(first.capturedAt)) / 3_600_000;
  const delta = last.usedPercent - first.usedPercent;
  if (elapsedHours < 1 / 12 || delta <= 0 || last.usedPercent >= 100) return null;
  const ratePerHour = delta / elapsedHours;
  const hoursRemaining = (100 - last.usedPercent) / ratePerHour;
  if (!Number.isFinite(hoursRemaining) || hoursRemaining <= 0 || hoursRemaining > horizonHours) return null;
  const roundedHours = Math.max(0.1, Math.round(hoursRemaining * 10) / 10);
  return {
    projectedAt: new Date(Date.parse(last.capturedAt) + hoursRemaining * 3_600_000).toISOString(),
    hoursRemaining: roundedHours,
    explanation: `Estimated from ${ordered.length} samples spanning ${Math.round(elapsedHours * 60)} minutes; current pace reaches 100% in about ${roundedHours}h.`,
    source: "estimated",
  };
}

export function metricSourceFromLedgerSource(source: string): MetricSource {
  if (source.startsWith("provider.")) return "reported";
  if (/estimate/i.test(source)) return "estimated";
  return "measured";
}

/** Join ledger rows to session metadata so history remains tied to work and outcomes. */
export function buildUsageHistory(rows: UsageLedgerRow[], sessions: Session[]): UsageHistoryEntry[] {
  const byId = new Map(sessions.map(session => [session.id, session]));
  return rows.map(row => {
    const session = row.sessionId ? byId.get(row.sessionId) : undefined;
    const tokenParts = [row.inputTokens, row.outputTokens].filter((value): value is number => value != null);
    const entry: UsageHistoryEntry = {
      id: row.id,
      workUnit: row.turnId ?? row.sessionId ?? row.workspaceId,
      harness: session?.harness ?? "unknown",
      model: session?.model ?? undefined,
      outcome: session?.status ?? "unknown",
      source: metricSourceFromLedgerSource(row.source),
      totalTokens: tokenParts.length ? tokenParts.reduce((total, value) => total + value, 0) : undefined,
      contextPercent: row.contextPercent ?? undefined,
      createdAt: row.createdAt,
    };
    return entry;
  }).sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt) || b.id - a.id);
}

/** Latest real usage snapshot from a session's live event stream, if any. */
export function latestUsageSnapshot(events: AgentEvent[]): UsageSnapshot | null {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    if (events[index].kind === "usage.updated") {
      const snapshot = extractUsageSnapshot(events[index].data);
      if (snapshot) return snapshot;
    }
  }
  return null;
}
