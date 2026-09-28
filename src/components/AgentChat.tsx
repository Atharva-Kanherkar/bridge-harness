import { useMemo } from "react";
import { Check, X } from "lucide-react";
import { bridgeApi } from "../api";
import { usePolledSessionForest } from "../forest";
import type { AgentEvent, ApprovalDecision, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import type { InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import { AgentConversation } from "./AgentConversation";
import { Markdown } from "./Markdown";
import { SteerComposer } from "./SteerComposer";
import { WorkerDiagnostics, reportedResult, workerDiagnostics, type ReportedResult } from "./WorkerControls";

// One agent's chat as the Agents pane shows it: the conversation Bridge draws
// for every chat, fed from the worker's own forest and its slice of the live
// stream, with the steer box where a chat keeps its composer. The pane used to
// open to a raw activity feed, frames printed as lines of mono text; this is
// the same messages, tool rows and approval cards the worker's own session
// view shows, so watching an agent from the dock loses nothing.
//
// One thing the chat cannot show on its own is the outcome. A worker reports
// inside a `bridge-worker-result` fence, which the transcript strips as wire
// chatter, so a worker that answers with only the fence would end on nothing.
// The typed result is drawn after the last row instead.

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
  const lastResult = runtime?.lastResult;
  const resultStatus = runtime?.resultStatus;
  // Stable across stream frames, so the conversation's memo is not defeated by
  // a card that has not changed.
  const trailing = useMemo(() => {
    const result = reportedResult(resultStatus, lastResult);
    return result ? <ResultCard result={result} /> : null;
  }, [lastResult, resultStatus]);
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
        trailing={trailing}
      />
    </div>
    {onSteer && <SteerComposer sessionId={session.id} steerable={steerable} onSteer={onSteer} className="shrink-0 border-t border-border px-3 py-2.5" />}
  </div>;
}

const RESULT_TONE: Record<string, { dot: string; text: string }> = {
  completed: { dot: "bg-success", text: "text-success" },
  failed: { dot: "bg-destructive", text: "text-destructive" },
};

/// The worker's typed result, as the last thing in its chat: status, summary,
/// and the files and checks it reports. Lists stay folded so a long run does
/// not bury the summary.
function ResultCard({ result }: { result: ReportedResult }) {
  const tone = RESULT_TONE[result.status ?? ""] ?? { dot: "bg-warning", text: "text-warning" };
  const failing = result.tests.filter(test => test.status === "failed").length;
  return <section aria-label="Worker result" className="min-w-0 overflow-hidden rounded-xl border border-border bg-card">
    <header className="flex items-center gap-2 px-3.5 pb-2 pt-2.5">
      <span className={cn("size-[7px] flex-none rounded-full", tone.dot)} aria-hidden="true" />
      <b className="text-[13px] font-medium text-foreground">Result</b>
      {result.status && <span className={cn("text-[10px] font-semibold uppercase tracking-[0.09em]", tone.text)}>{result.status.replaceAll("_", " ")}</span>}
    </header>
    {result.summary && <div className="border-t border-border px-3.5 py-2.5 text-[12px] leading-relaxed text-foreground/85"><Markdown text={result.summary} /></div>}
    {result.filesChanged.length > 0 && <details className="border-t border-border px-3.5 py-2 text-[11px] text-muted-foreground">
      <summary className="cursor-pointer">{result.filesChanged.length} file{result.filesChanged.length === 1 ? "" : "s"} changed</summary>
      <ul className="mt-1.5 space-y-0.5">{result.filesChanged.map(path => <li key={path} className="truncate font-mono text-foreground/80">{path}</li>)}</ul>
    </details>}
    {result.tests.length > 0 && <details className="border-t border-border px-3.5 py-2 text-[11px] text-muted-foreground">
      <summary className={cn("cursor-pointer", failing ? "text-destructive" : "text-success")}>{failing ? `${failing} of ${result.tests.length} checks failing` : `${result.tests.length} check${result.tests.length === 1 ? "" : "s"} passing`}</summary>
      <ul className="mt-1.5 space-y-0.5">{result.tests.map((test, index) => <li key={`${index}:${test.command}`} className="flex items-center gap-1.5 font-mono text-foreground/80">
        {test.status === "failed" ? <X size={11} className="shrink-0 text-destructive" aria-hidden="true" /> : <Check size={11} className="shrink-0 text-success" aria-hidden="true" />}
        <span className="min-w-0 truncate">{test.command}</span>
      </li>)}</ul>
    </details>}
  </section>;
}
