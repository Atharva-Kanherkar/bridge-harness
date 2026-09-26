import { useCallback, useState } from "react";

// Which agents the person pinned in the Agents pane. Pinned is what makes an
// agent sticky: it stays open and stays listed after it finishes, across chat
// switches and restarts, until it is unpinned. Session ids are global, so one
// list serves every chat; each chat only ever shows the ids that are its own.

export const PINNED_AGENTS_KEY = "bridge.agents.pinned";

export function readPinnedAgents(storage: Pick<Storage, "getItem"> = localStorage): Set<string> {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(PINNED_AGENTS_KEY) ?? "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string" && id !== "") : []);
  } catch {
    return new Set();
  }
}

export function writePinnedAgents(ids: ReadonlySet<string>, storage: Pick<Storage, "setItem"> = localStorage): void {
  try {
    storage.setItem(PINNED_AGENTS_KEY, JSON.stringify([...ids]));
  } catch {
    // Storage can be full or unavailable; a pin that does not survive a reload
    // is better than a pane that throws.
  }
}

export function usePinnedAgents(): [ReadonlySet<string>, (sessionId: string) => void] {
  const [pinned, setPinned] = useState<ReadonlySet<string>>(() => readPinnedAgents());
  const toggle = useCallback((sessionId: string) => {
    setPinned(current => {
      const next = new Set(current);
      if (next.has(sessionId)) next.delete(sessionId); else next.add(sessionId);
      writePinnedAgents(next);
      return next;
    });
  }, []);
  return [pinned, toggle];
}
