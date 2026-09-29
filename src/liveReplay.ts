import type { AgentEvent } from "./types";

/**
 * Gap-free live history for the webview.
 *
 * The live channel is bounded and notify-only: when a connection lags, the
 * daemon drops frames and owes a `stream-lagged` marker. The forest poll was
 * the only thing that ever put those frames back, and on a long session that
 * is a multi-megabyte snapshot taking seconds. Under load it fell behind far
 * enough that four finished replies never appeared. Durable frames carry a
 * per-session sequence, so a hole is visible the moment the next frame
 * arrives, and `replay_session_events` returns exactly the missing range in
 * one small page. This tracks one cursor per session and fills holes from it.
 */

export const REPLAY_PAGE_SIZE = 500;
/** Sessions replayed on a lag marker, most recently active first. */
export const LAG_REPLAY_SESSIONS = 8;
const MAX_TRACKED_SESSIONS = 256;

export interface LiveReplayDeps {
  replay: (sessionId: string, afterSequence: number, limit: number) => Promise<AgentEvent[]>;
  deliver: (events: AgentEvent[]) => void;
}

export interface LiveReplay {
  /** Every live frame, in arrival order. Starts a replay when one skips ahead. */
  observe(event: AgentEvent): void;
  /** The newest sequence a durable snapshot already holds for a session. */
  seed(sessionId: string, sequence: number): void;
  /** The live channel dropped frames: replay recent sessions from their cursors. */
  lagged(): void;
  /** Settles once no replay is in flight. For tests. */
  idle(): Promise<void>;
}

export function createLiveReplay({ replay, deliver }: LiveReplayDeps): LiveReplay {
  // Insertion order is recency: `touch` re-inserts, so the tail is the most
  // recently active session and eviction takes the head.
  const cursors = new Map<string, number>();
  const running = new Map<string, Promise<void>>();
  const again = new Set<string>();

  const touch = (sessionId: string, sequence: number) => {
    const current = cursors.get(sessionId);
    cursors.delete(sessionId);
    cursors.set(sessionId, current === undefined ? sequence : Math.max(current, sequence));
    while (cursors.size > MAX_TRACKED_SESSIONS) cursors.delete(cursors.keys().next().value!);
  };

  const fill = async (sessionId: string) => {
    for (;;) {
      const after = cursors.get(sessionId);
      if (after === undefined) return;
      const page = await replay(sessionId, after, REPLAY_PAGE_SIZE);
      const fresh = page.filter(event => event.sessionId === sessionId && event.sequence > after);
      if (fresh.length) {
        deliver(fresh);
        touch(sessionId, Math.max(...fresh.map(event => event.sequence)));
      }
      if (page.length < REPLAY_PAGE_SIZE || !fresh.length) return;
    }
  };

  const request = (sessionId: string) => {
    if (running.has(sessionId)) { again.add(sessionId); return; }
    const run = (async () => {
      try {
        do {
          again.delete(sessionId);
          await fill(sessionId);
        } while (again.has(sessionId));
      } catch {
        // Best effort, like the forest poll: the next gap or lag marker
        // retries, and the forest still converges on its own.
        again.delete(sessionId);
      } finally {
        running.delete(sessionId);
      }
    })();
    running.set(sessionId, run);
  };

  return {
    observe(event) {
      if (!(event.sequence > 0)) return;
      const cursor = cursors.get(event.sessionId);
      if (cursor === undefined) { touch(event.sessionId, event.sequence); return; }
      if (event.sequence <= cursor) return;
      // Contiguous: advance. A hole: leave the cursor where it is so the
      // replay starts from the last frame actually seen; it advances the
      // cursor past this frame when it lands.
      if (event.sequence === cursor + 1 && !running.has(event.sessionId)) touch(event.sessionId, event.sequence);
      else request(event.sessionId);
    },
    seed(sessionId, sequence) {
      if (sequence > 0) touch(sessionId, sequence);
    },
    lagged() {
      for (const sessionId of [...cursors.keys()].slice(-LAG_REPLAY_SESSIONS)) request(sessionId);
    },
    async idle() {
      while (running.size) await Promise.all(running.values());
    },
  };
}
