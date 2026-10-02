import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import { startSerialPoll } from "./polling";
import type { SessionForestSnapshot } from "./types";

// Heartbeats legitimately change worker runtime data every few seconds. Keep
// the durable entry array referentially stable when its contents did not
// change, so a heartbeat does not re-project the entire conversation. The
// forest is append-only, so equal length plus an equal last entry means an
// equal array — no stringify of the history is ever needed.
export function mergeForestSnapshot(
  current: SessionForestSnapshot | undefined,
  next: SessionForestSnapshot,
): SessionForestSnapshot {
  if (!current) return next;
  const last = current.entries[current.entries.length - 1];
  const nextLast = next.entries[next.entries.length - 1];
  const entriesUnchanged =
    current.entries.length === next.entries.length &&
    last?.id === nextLast?.id &&
    last?.sequence === nextLast?.sequence;
  return entriesUnchanged ? { ...next, entries: current.entries } : next;
}

/**
 * One session's forest, kept current by polling, for a surface that shows a
 * chat other than the selected one (an aside, a worker in the dock). Gated on
 * the cheap digest: the live stream can grow every frame during a turn, and a
 * full snapshot refetch on every frame is what used to hang those panels. The
 * snapshot itself is only refetched when the digest moves, or every tenth poll
 * as a backstop.
 */
export function usePolledSessionForest(sessionId: string, intervalMs = 3000): SessionForestSnapshot | undefined {
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const digestRef = useRef("");
  useEffect(() => {
    digestRef.current = "";
    setForest(undefined);
    let active = true;
    let pollsSinceFullFetch = 0;
    const refresh = async () => {
      const digest = await bridgeApi.sessionForestDigest(sessionId).catch(() => undefined);
      const force = pollsSinceFullFetch >= 9 || digest === undefined;
      if (!active) return;
      if (!force && digest === digestRef.current) {
        pollsSinceFullFetch += 1;
        return;
      }
      const value = await bridgeApi.sessionForest(sessionId).catch(() => undefined);
      if (!active) return;
      pollsSinceFullFetch = 0;
      if (!value) return;
      digestRef.current = digest ?? "";
      setForest(current => mergeForestSnapshot(current, value));
    };
    const stop = startSerialPoll(refresh, intervalMs);
    return () => { active = false; stop(); };
  }, [sessionId, intervalMs]);
  return forest;
}
