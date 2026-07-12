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
