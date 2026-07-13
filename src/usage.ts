import type { AgentEvent } from "./types";

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
}

export interface UsageSnapshot {
  windows: RateWindow[];
  contextPercent?: number;
  totalTokens?: number;
  costUsd?: number;
  planType?: string;
}

export type UsageProvider = "claude" | "codex";

export interface AccountUsagePayload {
  provider: UsageProvider;
  rateLimits: Record<string, unknown>;
}

type Dict = Record<string, unknown>;

function isDict(value: unknown): value is Dict {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function num(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "" && Number.isFinite(Number(value))) return Number(value);
  return undefined;
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

function windowFrom(id: string, value: Dict, nowMs: number): RateWindow | undefined {
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
  return { id, label, usedPercent, windowMinutes, resetsInSeconds, resetsLabel };
}

/** Parse a real usage snapshot from a `usage.updated` event payload. */
export function extractUsageSnapshot(data: unknown): UsageSnapshot | null {
  if (!isDict(data)) return null;
  const usage = isDict(data.usage) ? data.usage : undefined;

  const windows: RateWindow[] = [];
  const rateLimits = findRateLimits(data);
  if (rateLimits) {
    const nowMs = Date.now();
    for (const [id, value] of Object.entries(rateLimits)) {
      if (isDict(value)) {
        const window = windowFrom(id, value, nowMs);
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
  const snapshot: UsageSnapshot = { windows, contextPercent, totalTokens, costUsd, planType };
  const hasSignal = windows.length > 0 || contextPercent != null || totalTokens != null || costUsd != null;
  return hasSignal ? snapshot : null;
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
