import type { SessionStatus } from "./types";

export function safeSlug(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 42) || "task";
}

const transitions: Record<SessionStatus, SessionStatus[]> = {
  idle: ["working", "failed"], working: ["waiting", "ready", "stopped", "failed"],
  waiting: ["working", "stopped", "failed"], ready: ["working", "stopped"],
  stopped: ["working"], failed: ["working", "stopped"]
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
