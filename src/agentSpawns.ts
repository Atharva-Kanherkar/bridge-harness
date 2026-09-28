import { useEffect, useRef } from "react";

// Which agents a chat just started. The dock opens on the Agents pane when an
// orchestrator spawns one, so this has to tell a spawn apart from an agent that
// was already running when the chat was opened: the first reading for a chat
// only seeds what it has, and only ids that appear after that count.

export type SpawnWatch = { chatId: string; seen: ReadonlySet<string> };

export function readSpawns(previous: SpawnWatch | undefined, chatId: string, liveIds: readonly string[]): { next: SpawnWatch; spawned: string[] } {
  if (!previous || previous.chatId !== chatId) return { next: { chatId, seen: new Set(liveIds) }, spawned: [] };
  const spawned = liveIds.filter(id => !previous.seen.has(id));
  if (spawned.length === 0) return { next: previous, spawned };
  return { next: { chatId, seen: new Set([...previous.seen, ...spawned]) }, spawned };
}

/// Calls `onSpawn` with the newest agent each time `chatId` starts one.
export function useAgentSpawns(chatId: string | undefined, liveIds: readonly string[], onSpawn: (sessionId: string) => void) {
  const watch = useRef<SpawnWatch>();
  const latest = useRef(onSpawn);
  latest.current = onSpawn;
  const key = liveIds.join("\n");
  useEffect(() => {
    if (!chatId) {
      watch.current = undefined;
      return;
    }
    const { next, spawned } = readSpawns(watch.current, chatId, key ? key.split("\n") : []);
    watch.current = next;
    if (spawned.length > 0) latest.current(spawned[spawned.length - 1]);
  }, [chatId, key]);
}
