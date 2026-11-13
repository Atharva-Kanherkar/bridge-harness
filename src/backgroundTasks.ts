import { toolCallDisplay, type ConversationItem } from "./conversation";

/// Work an agent left running after its reply: a test run it backgrounded, a
/// watcher it armed. The model itself is idle and can be talked to, so this is
/// not the thinking row and the session is not `working`. The transcript
/// derives the list (it owns the forest projection); the composer shelf and
/// the Work pane read it through the small store at the bottom of this file.

export type BackgroundTaskKind = "running" | "watching";

export interface BackgroundTaskView {
  key: string;
  kind: BackgroundTaskKind;
  /** What the agent said the task is for, before any shell. */
  label: string;
  command?: string;
  elapsedMs?: number;
  /** One faint line of news: progress, the newest output line, or where output goes. */
  detail?: string;
  outputTail: string[];
  /** Files the command redirects its output into, by name. */
  logFiles: string[];
}

/// A provider's tool name is not a description of the work.
const GENERIC_TITLES = /^(bash|shell|monitor|task|command|terminal|local_bash|background update)$/i;
const TAIL_LINES = 6;

/** Files a shell command writes its output into, in the order it names them. */
export function redirectTargets(command?: string): string[] {
  if (!command) return [];
  const files = new Set<string>();
  for (const match of command.matchAll(/(?:^|[\s;(|])[0-9&]?>>?\s*([^\s;&|()<>]+)/g)) {
    const path = match[1].replace(/^["']|["']$/g, "");
    if (path && path !== "/dev/null") files.add(path);
  }
  return [...files];
}

const baseName = (path: string) => path.split("/").filter(Boolean).at(-1) ?? path;

export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/** The key a task keeps from launch to finish: a launcher and its task share it. */
export const backgroundTaskKey = (item: ConversationItem) => toolCallDisplay(item).sourceCallId ?? item.identity ?? item.key;

export function describeBackgroundTask(item: ConversationItem, now: number): BackgroundTaskView {
  const call = toolCallDisplay(item);
  const title = item.title?.trim();
  const label = (title && !GENERIC_TITLES.test(title) ? title : undefined)
    ?? call.intent ?? call.target ?? call.command ?? "Background task";
  const started = item.createdAt ? Date.parse(item.createdAt) : Number.NaN;
  const outputTail = (call.output ?? "").split("\n").map(line => line.trimEnd()).filter(line => line.trim()).slice(-TAIL_LINES);
  const logFiles = redirectTargets(call.command).map(baseName);
  const news = call.progress ?? outputTail.at(-1) ?? (logFiles.length ? `Writing to ${logFiles.at(-1)}` : undefined);
  return {
    key: backgroundTaskKey(item),
    kind: call.passive ? "watching" : "running",
    label,
    command: call.command,
    elapsedMs: Number.isFinite(started) ? Math.max(0, now - started) : call.durationMs,
    detail: news && news !== label ? news : undefined,
    outputTail,
    logFiles,
  };
}

/// How a task that left the list ended, when the runtime said so. A launch
/// acknowledgement, a lost task or an unknown outcome announces nothing: no
/// surface ever claims a result it was not given.
export type BackgroundOutcome = "done" | "failed" | "stopped";

export function backgroundOutcome(item: ConversationItem): BackgroundOutcome | undefined {
  const call = toolCallDisplay(item);
  if (call.status === "running") return undefined;
  if (call.status === "failed" || (call.exitCode !== undefined && call.exitCode !== 0)) return "failed";
  if (item.status === "stopped") return "stopped";
  return call.status === "completed" && !call.statusLabel ? "done" : undefined;
}

// ── the store ───────────────────────────────────────────────────────────────

/// What the open chat's transcript reports about its background work. Only the
/// main chat publishes, so one slot is enough; `scope` is its session id, and a
/// reader asking for any other scope sees nothing.
export type BackgroundWork = {
  scope: string;
  items: ConversationItem[];
  /** Changes whenever a task starts, ends or prints: the store's only trigger. */
  signature: string;
  /** How a task that just left `items` ended, by its key. */
  outcomeOf: (key: string) => BackgroundOutcome | undefined;
};

export type BackgroundWorkStore = {
  publish: (work: BackgroundWork) => void;
  read: () => BackgroundWork | null;
  subscribe: (listener: () => void) => () => void;
};

/// A same-signature publish (every transcript render) swaps in the newest
/// `outcomeOf` without waking anyone: the transcript re-renders on every
/// streamed token, and the composer must not.
export function createBackgroundWorkStore(): BackgroundWorkStore {
  let current: BackgroundWork | null = null;
  const listeners = new Set<() => void>();
  return {
    publish(work) {
      const changed = !current || current.scope !== work.scope || current.signature !== work.signature;
      if (!changed && current) { current.outcomeOf = work.outcomeOf; return; }
      current = { ...work };
      for (const listener of listeners) listener();
    },
    read: () => current,
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

export const backgroundWork = createBackgroundWorkStore();
