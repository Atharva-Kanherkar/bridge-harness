import { useMemo } from "react";
import { bridgeApi } from "../api";
import { usePolledSessionForest } from "../forest";
import type { AgentEvent, ApprovalDecision, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import type { InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { AgentConversation } from "./AgentConversation";
import { SteerComposer } from "./SteerComposer";
import { WorkerDiagnostics, workerDiagnostics } from "./WorkerControls";

// One agent's chat as the Agents pane shows it: the conversation Bridge draws
// for every chat, fed from the worker's own forest and its slice of the live
// stream, with the steer box where a chat keeps its composer. The pane used to
// open to a raw activity feed, frames printed as lines of mono text; this is
// the same messages, tool rows and approval cards the worker's own session
// view shows, so watching an agent from the dock loses nothing.

export function AgentChat({ session, runtime, liveEvents, reasons = [], onOpenSession, onSteer }: {
  session: Session;
  runtime?: WorkerRuntimeRecord;
  /** The global live stream; the worker's frames arrive under its own id. */
  liveEvents: AgentEvent[];
  reasons?: BridgeEvent[];
  onOpenSession?: (sessionId: string) => void;
  /** Send guidance into this worker. Omitted where steering is not offered. */
  onSteer?: (sessionId: string, text: string) => Promise<void>;
}) {
  const forest = usePolledSessionForest(session.id);
  const events = useMemo(() => liveEvents.filter(event => event.sessionId === session.id), [liveEvents, session.id]);
  const working = session.status === "working" || session.activeTurnId != null;
  // Mirrors the backend gate (session_input::worker_steer_gate) so the box is
  // not offered for a steer that would be refused.
  const steerable = runtime?.resultStatus !== "reported"
    && runtime?.lifecycleState !== "checkpointing"
    && (session.status === "working" || session.status === "waiting");
  const hasDiagnostics = workerDiagnostics(reasons, session.id).length > 0;
  // A worker's approval card is resolvable right here: it is the one place the
  // person is looking while the worker waits on them.
  const resolve = (eventId: number, decision: ApprovalDecision, optionId?: string): Promise<InteractionResolutionResult | void> => bridgeApi.resolveApproval(session.id, eventId, decision, optionId);
  const answer = (eventId: number, action: QuestionAction, answers: Record<string, string[]>): Promise<InteractionResolutionResult | void> => bridgeApi.resolveQuestion(session.id, eventId, action, answers);

  return <div role="region" aria-label={`Agent ${session.title || session.label}`} className="flex h-full min-h-0 flex-col">
    {hasDiagnostics && <div className="shrink-0 border-b border-border px-3 py-2"><WorkerDiagnostics reasons={reasons} sessionId={session.id} /></div>}
    <div className="relative min-h-0 flex-1 overflow-hidden">
      <AgentConversation
        session={session}
        events={events}
        forestEntries={forest?.entries}
        entryWindow={forest?.entryWindow}
        activeLeafId={forest?.head?.activeEntryId}
        continuationFidelity={session.continuationFidelity}
        working={working}
        onResolve={resolve}
        onAnswerQuestion={answer}
        onOpenSession={onOpenSession}
        preview={false}
        density="compact"
      />
    </div>
    {onSteer && <SteerComposer sessionId={session.id} steerable={steerable} onSteer={onSteer} className="shrink-0 border-t border-border px-3 py-2.5" />}
  </div>;
}
