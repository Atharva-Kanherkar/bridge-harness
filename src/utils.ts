import type { CapabilityTier, SessionStatus } from "./types";

const MODEL_LABELS: Record<string, string> = { "gpt-5.6-luna": "GPT Luna", "gpt-5.6-terra": "GPT Terra", "gpt-5.6-sol": "GPT Sol", "gpt-5.3-codex": "GPT-5.3 Codex", sonnet: "Sonnet", opus: "Opus", haiku: "Haiku", fable: "Fable" };

export const modelLabel = (model?: string | null) => (model ? MODEL_LABELS[model] ?? model : "—");

export function tierRuntimeLabel(tier?: CapabilityTier | null, model?: string | null, effort?: string | null): string {
  const routing = tier ? `${tier.toUpperCase()} TIER` : "TIER —";
  return `${routing}${effort ? ` · ${effort}` : ""} · runtime ${modelLabel(model)}`;
}

const BUILTIN_HARNESS_LABELS: Record<string, string> = { claude: "Claude", codex: "Codex", opencode: "OpenCode", shell: "Shell" };

/**
 * Display name for a harness id.
 *
 * The id space is open and names the *agent*, never how Bridge runs it. An
 * agent Bridge has no bespoke label for is shown under the id it was installed
 * by — never relabelled as something else.
 *
 * One implementation on purpose: three near-copies of this used to live in
 * App, the sidebar, and the conversation view, which is how the next harness
 * ends up mislabelled in two of the three.
 */
export function harnessLabel(harness?: string | null): string {
  if (!harness) return "Agent";
  return BUILTIN_HARNESS_LABELS[harness] ?? harness[0].toUpperCase() + harness.slice(1);
}

export function safeSlug(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 42) || "task";
}

const transitions: Record<SessionStatus, SessionStatus[]> = {
  idle: ["starting", "working", "failed"],
  starting: ["working"],
  working: ["waiting", "warm", "completed", "checkpointing", "ready", "stopped", "failed", "cancelled"],
  waiting: ["working", "stopped", "failed", "cancelled"],
  warm: ["working", "checkpointing"],
  checkpointing: ["stopped"],
  ready: ["working", "stopped"],
  stopped: ["resuming", "working"],
  resuming: ["working", "restored"],
  restored: ["working"],
  failed: ["resuming", "completed", "working", "stopped"],
  completed: [],
  cancelled: []
};

export function canTransition(from: SessionStatus, to: SessionStatus): boolean {
  return from === to || transitions[from].includes(to);
}

export function formatElapsed(startedAt: string | null | undefined, now = Date.now()): string {
  if (!startedAt) return "—";
  const elapsed = Math.max(0, now - new Date(startedAt).getTime());
  if (!Number.isFinite(elapsed)) return "—";
  const minutes = Math.floor(elapsed / 60_000);
  const hours = Math.floor(minutes / 60);
  return hours ? `${hours}h ${String(minutes % 60).padStart(2, "0")}m` : `${minutes}m`;
}
