import { isToolItem, reduceConversation, toolCallDisplay } from "./conversation";
import { isToolLive, type ToolCallDisplay } from "./transcript/toolCall";
import { readWireKind } from "./transcript/wire";
import type { AgentEvent, Session } from "./types";
import { byRecency, type LiveAgents } from "./components/sidebarChats";

// Which chats the sidebar's Running group lists, and what each row says. A
// view, not a move: every chat here is still in its project folder too.

/**
 * Whether the chat's own turn is in flight.
 *
 * Not `status === "working"`: that status also covers a provider that is up
 * with nothing to do, and a chat that sat idle under it would read as running
 * for good. A turn is in flight once the provider has acknowledged it
 * (`activeTurnId`, the field the backend's `turn.started` handler writes), or
 * while a send to it is still on its way: the same two signals the
 * transcript's `turnActive` reads before it falls back to the status.
 */
export function turnInFlight(chat: Pick<Session, "activeTurnId">, sending = false): boolean {
  return !!chat.activeTurnId || sending;
}

export type RunningChat = {
  chat: Session;
  /** Epoch ms this run started, as well as this view knows it. */
  since: number;
  /** The chat's own turn is in flight. */
  turn: boolean;
  /** Live workers anywhere under the chat. */
  workers: number;
  /** Of those, the ones waiting on the person. */
  workersWaiting: number;
  /** The chat itself is waiting on the person. */
  needsYou: boolean;
  /** The step in flight, in the transcript's own words. Null when there is none. */
  action: string | null;
};

export type RunningInput = {
  /** The top-level chats the sidebar lists. */
  chats: readonly Session[];
  /** Each chat's live descendant agents (`liveAgentSessions`). */
  liveAgents?: LiveAgents;
  /** Every session, workers included: their states. Without it a live worker counts as working. */
  sessions?: readonly Session[];
  /** Chats with a send still on its way. */
  sending?: ReadonlySet<string>;
  /** The live event buffer, every session's frames. */
  events?: readonly AgentEvent[];
  /** When this view first saw each chat running: the clock for a run whose start the live stream no longer holds. */
  seen?: ReadonlyMap<string, number>;
  now: number;
};

/** Every chat with a turn or a worker in flight, the newest run first. */
export function runningChats({ chats, liveAgents, sessions, sending, events = [], seen, now }: RunningInput): RunningChat[] {
  const byId = sessions ? new Map(sessions.map(session => [session.id, session])) : undefined;
  const rows: RunningChat[] = [];
  for (const chat of chats) {
    const turn = turnInFlight(chat, sending?.has(chat.id) ?? false);
    const live = liveAgents?.get(chat.id) ?? [];
    if (!turn && live.length === 0) continue;
    const workers = live.map(id => byId?.get(id)).filter((worker): worker is Session => !!worker);
    const own = turn ? sessionEvents(events, chat.id) : [];
    const known = seen?.get(chat.id);
    const since = turn
      ? turnStartedAt(own) ?? known ?? now
      : known ?? earliestStart(workers) ?? now;
    rows.push({
      chat,
      since,
      turn,
      workers: live.length,
      workersWaiting: workers.filter(worker => worker.status === "waiting").length,
      needsYou: chat.status === "waiting",
      action: turn ? liveAction(own) : null,
    });
  }
  return rows.sort((a, b) => b.since - a.since || byRecency(a.chat, b.chat) || a.chat.id.localeCompare(b.chat.id));
}

/** The clocks to carry into the next pass: one per chat still running, gone once it stops. */
export function rememberRunning(rows: readonly RunningChat[]): Map<string, number> {
  return new Map(rows.map(row => [row.chat.id, row.since]));
}

export type RunningDetail = { text: string; tone: "live" | "needsYou" };

/** The row's second line after the timer: who needs the person, else the step, else the workers. */
export function runningDetail(row: RunningChat): RunningDetail {
  if (row.needsYou) return { text: "Needs you", tone: "needsYou" };
  if (row.workersWaiting > 0) {
    return { text: `${row.workersWaiting} ${row.workersWaiting === 1 ? "worker needs" : "workers need"} you`, tone: "needsYou" };
  }
  if (row.action) return { text: row.action, tone: "live" };
  if (row.workers > 0) return { text: `${row.workers} ${row.workers === 1 ? "worker" : "workers"} working`, tone: "live" };
  return { text: "Working", tone: "live" };
}

/** `0:13`, `12:05`, `1:02:03`: a stopwatch, not a duration in words. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor((Number.isFinite(ms) ? ms : 0) / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`;
}

/**
 * One session's frames from its newest turn on. Older turns cannot hold the
 * step in flight, and leaving them out keeps the fold small on a long chat.
 */
function sessionEvents(events: readonly AgentEvent[], sessionId: string): AgentEvent[] {
  const own = events.filter(event => event.sessionId === sessionId);
  for (let index = own.length - 1; index >= 0; index--) {
    if (readWireKind(own[index].kind) === "turn.started") return own.slice(index);
  }
  return own;
}

/**
 * When the turn in flight started, read off its `turn.started` frame. Null when
 * the buffer no longer holds it, or when the newest turn frame it does hold
 * ended a turn: that start belonged to an earlier one.
 */
export function turnStartedAt(events: readonly AgentEvent[]): number | null {
  for (let index = events.length - 1; index >= 0; index--) {
    const kind = readWireKind(events[index].kind);
    if (!kind.startsWith("turn.")) continue;
    if (kind !== "turn.started") return null;
    const at = Date.parse(events[index].createdAt);
    return Number.isFinite(at) ? at : null;
  }
  return null;
}

function earliestStart(sessions: readonly Session[]): number | null {
  const starts = sessions.map(session => Date.parse(session.startedAt ?? "")).filter(Number.isFinite);
  return starts.length ? Math.min(...starts) : null;
}

/**
 * What a live call is doing, worded the way the transcript's live card words
 * it. A command the transcript shortened to its first words keeps them, less
 * a dangling flag: `cargo test -p` reads as `cargo test`.
 */
export function actionLabel(call: ToolCallDisplay): string {
  const shortened = !!call.command && !!call.target && call.command !== call.target && call.command.startsWith(call.target);
  const target = shortened ? call.target!.replace(/(\s+-\S*)+$/, "") : call.target;
  if (call.passive) return `Watching${target ? ` ${target}` : ""}`;
  return call.intent ?? `${call.doing}${target ? ` ${target}` : ""}`;
}

/** The newest call still running in a turn that is in flight. */
export function liveAction(events: readonly AgentEvent[]): string | null {
  if (!events.length) return null;
  const items = reduceConversation([...events]);
  for (let index = items.length - 1; index >= 0; index--) {
    const item = items[index];
    if (!isToolItem(item)) continue;
    const call = toolCallDisplay(item);
    if (isToolLive(call, true)) return actionLabel(call);
  }
  return null;
}
