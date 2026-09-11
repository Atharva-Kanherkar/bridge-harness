import { useEffect, useReducer, useRef } from "react";
import type { Session } from "./types";

type StopSession = Pick<Session, "id" | "status" | "activeTurnId">;
const delivered = (session: StopSession) => !!session.activeTurnId || ["working", "waiting", "checkpointing"].includes(session.status);

/** Requests belong to session IDs, including requests made before launch has
 * acknowledged a turn. Switching the selected chat never retargets a stop. */
export function createSessionStops(interrupt: (id: string) => Promise<void>, changed: () => void, failed: (id: string, error: unknown) => void) {
  const requests = new Map<string, { sent: boolean; acknowledged: boolean }>();
  let disposed = false;
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
      requests.delete(id);
      failed(id, error);
      changed();
    });
  }
  return {
    has: (id?: string) => !!id && requests.has(id),
    request(session: StopSession) {
      if (disposed || requests.has(session.id)) return;
      requests.set(session.id, { sent: false, acknowledged: false });
      changed();
      if (delivered(session)) send(session.id);
    },
    reconcile(sessions: StopSession[], pending: ReadonlySet<string>) {
      let removed = false;
      for (const [id, request] of requests) {
        const session = sessions.find(session => session.id === id);
        if (session && delivered(session)) { send(id); continue; }
        if (!pending.has(id) && (!request.sent || request.acknowledged || !session || session.status === "stopped")) {
          requests.delete(id);
          removed = true;
        }
      }
      if (removed) changed();
    },
    activate() { disposed = false; },
    dispose() { disposed = true; requests.clear(); },
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
