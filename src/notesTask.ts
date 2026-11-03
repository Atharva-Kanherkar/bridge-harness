import type { Note, Session, SessionStatus } from "./types";

const CHAT_STATUS: Record<SessionStatus, string> = {
  idle: "Chat started",
  starting: "Starting…",
  working: "Running",
  waiting: "Needs you",
  warm: "Running",
  checkpointing: "Saving progress…",
  ready: "Ready to review",
  stopped: "Stopped",
  resuming: "Resuming…",
  restored: "Resuming…",
  failed: "Failed",
  completed: "Finished",
  cancelled: "Cancelled",
};

/** A finished chat turn is ready for review; only the person marks the task done. */
export function noteTaskStatus(task: Note, session?: Session): string {
  if (task.state === "done") return "Done";
  if (task.state === "archived") return "Archived";
  if (session) return CHAT_STATUS[session.status] ?? "Chat linked";
  if (task.sessionIds.length) return "Chat linked";
  return task.state === "doing" ? "In progress" : "Not started";
}

/** The newest link stays authoritative even before its session has loaded. */
export function latestTaskChat(task: Note, sessions: ReadonlyMap<string, Session>) {
  const id = task.sessionIds.at(-1);
  return { id, session: id ? sessions.get(id) : undefined };
}
