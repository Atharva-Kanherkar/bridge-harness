import { useEffect, useReducer, useRef } from "react";
import type { Session } from "./types";

type StopSession = Pick<Session, "id" | "status" | "activeTurnId">;
const delivered = (session: StopSession) => !!session.activeTurnId || ["working", "waiting", "checkpointing"].includes(session.status);

/** Requests belong to session IDs, including requests made before launch has
 * acknowledged a turn. Switching the selected chat never retargets a stop. */
export function createSessionStops(interrupt: (id: string) => Promise<void>, changed: () => void, failed: (id: string, error: unknown) => void) {
  type Send = { cancelled: boolean; finished: boolean; phase: "preparing" | "starting" | "submitted"; withdrawn?: () => void };
  type Request = { sent: boolean; acknowledged: boolean; captured: Set<Send>; settled: Promise<void>; release: () => void };
  const requests = new Map<string, Request>();
  const sends = new Map<string, Set<Send>>();
  let disposed = false;
  function remove(id: string) { requests.get(id)?.release(); requests.delete(id); }
  function send(id: string) {
    const request = requests.get(id);
    if (!request || request.sent) return;
    request.sent = true;
    void interrupt(id).then(() => {
      if (disposed || requests.get(id) !== request) return;
      request.acknowledged = true;
      changed();
    }, error => {
      if (disposed || requests.get(id) !== request) return;
      remove(id);
      failed(id, error);
      changed();
    });
  }
  return {
    has: (id?: string) => !!id && requests.has(id),
    /** `withdrawn` runs synchronously when Stop lands before submission: the
     * words never reached the backend, so the sender can hand them back. */
    beginSend(id: string, withdrawn?: () => void) {
      const entry: Send = { cancelled: false, finished: false, phase: "preparing", withdrawn };
      const active = sends.get(id) ?? new Set<Send>();
      active.add(entry); sends.set(id, active);
      return {
        ready: requests.get(id)?.settled,
        get cancelled() { return disposed || entry.cancelled; },
        get withdrawn() { return entry.cancelled && entry.phase !== "submitted"; },
        starting() { entry.phase = "starting"; },
        submitted() { entry.phase = "submitted"; },
        stopIfCancelled() {
          if (entry.cancelled && requests.get(id)?.captured.has(entry)) send(id);
        },
        finish() {
          entry.finished = true; active.delete(entry);
          if (!active.size) sends.delete(id);
          if (!disposed) changed();
        },
      };
    },
    request(session: StopSession) {
      if (disposed || requests.has(session.id)) return;
      const captured = new Set(sends.get(session.id));
      for (const pending of captured) {
        pending.cancelled = true;
        if (pending.phase !== "submitted") pending.withdrawn?.();
      }
      // Preparation is local work we can invalidate immediately. No provider
      // turn needs to be launched merely to make it possible to stop it.
      if (!delivered(session) && captured.size && [...captured].every(item => item.phase === "preparing")) {
        changed(); return;
      }
      let release!: () => void;
      const settled = new Promise<void>(resolve => { release = resolve; });
      requests.set(session.id, { sent: false, acknowledged: false, captured, settled, release });
      changed();
      if (delivered(session)) send(session.id);
    },
    reconcile(sessions: StopSession[], pending: ReadonlySet<string>) {
      let removed = false;
      for (const [id, request] of requests) {
        const session = sessions.find(session => session.id === id);
        if (session && delivered(session)) { send(id); continue; }
        const pendingSend = request.captured.size
          ? [...request.captured].some(item => !item.finished)
          : pending.has(id);
        if ((session?.status === "stopped" || !pendingSend) && (!request.sent || request.acknowledged || !session || session.status === "stopped")) {
          remove(id);
          removed = true;
        }
      }
      if (removed) changed();
    },
    activate() { disposed = false; },
    dispose() { disposed = true; for (const id of requests.keys()) remove(id); for (const active of sends.values()) for (const entry of active) entry.cancelled = true; sends.clear(); },
  };
}

export function useSessionStops(sessions: Session[], pendingIds: ReadonlySet<string>, interrupt: (id: string) => Promise<void>, failed: (id: string, error: unknown) => void) {
  const [revision, changed] = useReducer(value => value + 1, 0);
  const callbacks = useRef({ interrupt, failed });
  callbacks.current = { interrupt, failed };
  const controller = useRef<ReturnType<typeof createSessionStops>>();
  if (!controller.current) controller.current = createSessionStops(id => callbacks.current.interrupt(id), changed, (id, error) => callbacks.current.failed(id, error));
  useEffect(() => controller.current!.reconcile(sessions, pendingIds), [sessions, pendingIds, revision]);
  // Requests are scoped to this mounted app. Promise callbacks never touch a
  // newer controller after teardown.
  useEffect(() => { controller.current!.activate(); return () => controller.current!.dispose(); }, []);
  return controller.current;
}
