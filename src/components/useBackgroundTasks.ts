import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  backgroundWork,
  describeBackgroundTask,
  type BackgroundOutcome,
  type BackgroundTaskView,
  type BackgroundWorkStore,
} from "../backgroundTasks";
import type { ConversationItem } from "../conversation";

/// One line of news after a task leaves the list: which task, and how it ended.
export type BackgroundNotice = { key: string; label: string; outcome: BackgroundOutcome; more: number };

const NO_ITEMS: ConversationItem[] = [];
export const NOTICE_MS = 4000;

function useClock(intervalMs: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    if (intervalMs === null) return;
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/**
 * The open chat's background tasks, read from what its transcript published.
 * Another chat's tasks are never returned, and the clock only ticks while
 * something is running. A task that leaves the list with an outcome the
 * runtime reported becomes a notice for a few seconds; a launch
 * acknowledgement, a lost task or a chat switch announces nothing.
 */
export function useBackgroundTasks(scope: string | undefined, store: BackgroundWorkStore = backgroundWork) {
  const work = useSyncExternalStore(store.subscribe, store.read);
  const items = work && scope && work.scope === scope ? work.items : NO_ITEMS;
  const now = useClock(items.length ? 1000 : null);
  const tasks = useMemo(() => items.map(item => describeBackgroundTask(item, now)), [items, now]);
  const [notice, setNotice] = useState<BackgroundNotice | null>(null);

  const previous = useRef<{ scope?: string; labels: Map<string, string> }>({ scope, labels: new Map() });
  useEffect(() => {
    const labels = new Map(tasks.map(task => [task.key, task.label]));
    const before = previous.current;
    previous.current = { scope, labels };
    if (before.scope !== scope) { setNotice(null); return; }
    const outcomeOf = store.read()?.scope === scope ? store.read()?.outcomeOf : undefined;
    const finished = [...before.labels].flatMap(([key, label]) => {
      if (labels.has(key)) return [];
      const outcome = outcomeOf?.(key);
      return outcome ? [{ key, label, outcome }] : [];
    });
    if (!finished.length) return;
    const shown = finished.find(task => task.outcome === "failed") ?? finished[0];
    setNotice({ ...shown, more: finished.length - 1 });
  }, [tasks, scope, store]);

  useEffect(() => {
    if (!notice) return;
    const id = setTimeout(() => setNotice(null), NOTICE_MS);
    return () => clearTimeout(id);
  }, [notice]);

  return { tasks, notice } as { tasks: BackgroundTaskView[]; notice: BackgroundNotice | null };
}
