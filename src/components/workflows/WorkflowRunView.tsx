import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, Bell, Check, LoaderCircle, MessageSquare, ShieldCheck, Square, X } from "lucide-react";
import { bridgeApi } from "../../api";
import type { AgentEvent } from "../../types";
import { readWireKind } from "../../transcript/wire";
import type { ForkSessionResult, Workflow, WorkflowRun } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { ScreenHeading } from "../ui/screen";
import { ACTIVE_RUN_STATUSES, RUN_STATUS_LABEL, relativeTime, runTone } from "./format";
import { Card, ErrorNote, StatusChip, errorText } from "./parts";

const TRIGGER_LABEL: Record<WorkflowRun["triggerKind"], string> = { manual: "Run now", schedule: "schedule", catch_up: "catch-up" };

type Step = { key: string; icon: "ok" | "denied" | "ask" | "say" | "wait"; title: string; detail: string; identity?: string; pending?: AgentEvent };

/** `mcp__claude_ai_Slack__slack_send_message` reads as `Slack · slack_send_message`. */
export function toolLabel(identity: string): string {
  const match = /^mcp__(.+?)__(.+)$/.exec(identity);
  if (!match) return identity;
  return `${match[1].replace(/^claude_ai_/, "").replace(/_/g, " ")} · ${match[2]}`;
}

/** What Bridge or the person decided about a write, in words. */
function decisionText(resolution: Record<string, unknown>): string {
  const byWorkflow = resolution.resolvedBy === "workflow";
  const reason = typeof resolution.reason === "string" ? resolution.reason : "";
  if (resolution.decision === "accept") return byWorkflow ? "always allowed for this workflow" : "allowed by you";
  if (byWorkflow && reason.startsWith("dry run")) return "dry run: recorded as a preview, not sent";
  return byWorkflow ? reason || "declined by Bridge" : "declined by you";
}

/** The agent's text without the machine-read result block: the summary shows it. */
function withoutResultBlock(text: string): string {
  return text.replace(/\n?```workflow-result[\s\S]*?```/g, "").trim();
}

/**
 * The run's transcript, reduced to what a person reviewing it needs: one row
 * per tool call (its events share an `itemId`), saying what was decided and
 * whether it ran, and what the agent said.
 */
export function runSteps(events: AgentEvent[], active = true): Step[] {
  const resolutions = new Map<number, Record<string, unknown>>();
  for (const event of events) {
    const kind = readWireKind(event.kind);
    if (kind === "permission.resolved" || kind === "permission.resolving" || kind === "approval.resolved") {
      const data = event.data as Record<string, unknown>;
      const id = Number(data.requestEventId ?? data.eventId);
      if (Number.isFinite(id)) resolutions.set(id, data);
    }
  }
  type Call = Step & { decided?: string; outcome?: "ran" | "failed"; failure?: string };
  const steps: Call[] = [];
  const calls = new Map<string, Call>();
  const call = (event: AgentEvent, identity: string): Call => {
    const key = event.itemId ?? `s${event.sequence}`;
    let found = calls.get(key);
    if (!found) {
      found = { key, icon: "wait", title: toolLabel(identity), identity, detail: "" };
      calls.set(key, found);
      steps.push(found);
    }
    return found;
  };
  for (const event of events) {
    const data = event.data as Record<string, unknown>;
    const kind = readWireKind(event.kind);
    if (kind === "tool.started") {
      call(event, String(data.toolName ?? data.name ?? event.title ?? "tool"));
    } else if (kind === "permission.requested") {
      const current = call(event, String(data.tool_name ?? event.title ?? "tool"));
      const resolution = resolutions.get(event.sequence) ?? (data.resolvedBy ? data : undefined);
      if (resolution) current.decided = decisionText(resolution);
      else if (event.status === "settling" || event.status === "resolved") current.decided = "decided";
      else current.pending = event;
    } else if (kind === "tool.completed" && event.itemId && calls.has(event.itemId)) {
      const current = calls.get(event.itemId)!;
      current.outcome = event.status === "failed" ? "failed" : "ran";
      current.failure = event.text ?? undefined;
    } else if ((kind === "assistant.message" || kind === "message.completed") && (event.role ?? "assistant") === "assistant" && event.text) {
      const text = withoutResultBlock(event.text);
      if (text) steps.push({ key: `m${event.sequence}`, icon: "say", title: "Agent", detail: text });
    }
  }
  return steps.map(({ decided, outcome, failure, ...step }) => {
    if (step.icon === "say") return step;
    if (step.pending) return { ...step, icon: "ask", detail: "write: asking you" };
    if (outcome === "ran") return { ...step, icon: "ok", detail: decided ? `${decided}, then ran` : "ran" };
    if (outcome === "failed" || decided) return { ...step, icon: "denied", detail: decided ?? failure ?? "failed" };
    return { ...step, detail: active ? "running" : "did not finish" };
  });
}

export function WorkflowRunView({ workflow, run, onBack, onContinue }: {
  workflow: Workflow;
  run: WorkflowRun;
  onBack: () => void;
  onContinue: (result: ForkSessionResult) => void;
}) {
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = ACTIVE_RUN_STATUSES.includes(run.status);

  const load = useCallback(async () => {
    if (!run.sessionId) return;
    try {
      setEvents(await bridgeApi.replaySessionEvents(run.sessionId, 0, 500));
    } catch (reason) {
      setError(errorText(reason));
    }
  }, [run.sessionId]);

  useEffect(() => { void load(); }, [load, run.status]);
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(timer);
  }, [active, load]);

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
      await load();
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setBusy(false);
    }
  };

  const continueInChat = () => act(async () => {
    if (!run.sessionId) throw new Error("This run has no conversation to continue.");
    const forest = await bridgeApi.sessionForest(run.sessionId);
    const last = [...forest.entries].sort((a, b) => b.sequence - a.sequence)[0];
    if (!last) throw new Error("This run has no conversation to continue.");
    onContinue(await bridgeApi.forkSession(run.sessionId, last.id, `${workflow.definition.name} (continued)`, null, null, "shared"));
  });

  const steps = runSteps(events, active);
  const when = run.startedAt ?? run.createdAt;
  const tokens = run.tokens ? `${run.tokens.toLocaleString()} tokens` : null;
  const meta = [workflow.definition.projectId ? null : "No project", TRIGGER_LABEL[run.triggerKind], run.dryRun ? "dry run" : null,
    run.missedSlots > 0 ? `${run.missedSlots} missed slot${run.missedSlots === 1 ? "" : "s"}` : null, relativeTime(when, Date.now()), tokens,
    run.quotaUnknown ? "quota unknown" : null].filter(Boolean).join(" · ");

  return <div>
    <button type="button" onClick={onBack} className="mb-3 inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground"><ArrowLeft size={12}/>{workflow.definition.name}</button>
    <ScreenHeading title={`Run · ${new Date(when).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}`}
      description={<span className="inline-flex flex-wrap items-center gap-2"><StatusChip label={RUN_STATUS_LABEL[run.status]} tone={runTone(run.status)}/><span>{meta}</span></span>}
      action={<>
        {active && <Button size="xs" variant="ghost" disabled={busy} onClick={() => void act(() => bridgeApi.cancelWorkflowRun(run.id))}><Square size={11}/>Stop</Button>}
        <Button size="xs" variant="secondary" disabled={busy || !run.sessionId} onClick={() => void continueInChat()}><MessageSquare size={11}/>Continue in chat</Button>
      </>}/>
    {run.summary && <Card className="mb-3"><p className="text-[13px] leading-relaxed text-foreground">{run.summary}</p>{run.failureCode && <p className="mt-1 font-mono text-caption text-muted-foreground">{run.failureCode}</p>}</Card>}
    <div className="rounded-xl border border-border bg-card px-4 py-2">
      {!run.sessionId && <p className="py-3 text-caption text-muted-foreground">{run.status === "skipped" ? "Skipped: another run of this workflow was still going." : "This run has not started a conversation."}</p>}
      {run.sessionId && steps.length === 0 && <p className="flex items-center gap-2 py-3 text-caption text-muted-foreground">{active && <LoaderCircle className="animate-spin" size={12}/>}{active ? "Working…" : "Nothing was recorded for this run."}</p>}
      <div className="divide-y divide-border">{steps.map(step => <div key={step.key} className="flex items-start gap-3 py-2.5">
        <div className={`mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-md border ${step.icon === "ask" ? "border-foreground text-foreground" : "border-border text-muted-foreground"}`}>
          {step.icon === "ok" ? <Check size={12}/> : step.icon === "denied" ? <X size={12}/> : step.icon === "ask" ? <Bell size={12}/> : step.icon === "wait" ? <LoaderCircle size={12} className={active ? "animate-spin" : ""}/> : <MessageSquare size={12}/>}
        </div>
        <div className="min-w-0 flex-1">
          <p className={`${step.icon === "say" ? "font-sans" : "font-mono"} text-[12px] text-foreground`}>{step.title}</p>
          <p className="mt-0.5 whitespace-pre-wrap break-words text-caption text-muted-foreground">{step.detail}</p>
          {step.pending && <div className="mt-2.5 flex flex-wrap gap-1.5">
            <Button size="xs" disabled={busy} onClick={() => void act(() => bridgeApi.resolveApproval(run.sessionId!, step.pending!.sequence, "accept"))}><Check size={10}/>Allow once</Button>
            <Button size="xs" variant="secondary" disabled={busy} onClick={() => void act(() => bridgeApi.alwaysAllowWorkflowTool(workflow.id, run.sessionId!, step.pending!.sequence, step.identity ?? step.title))}><ShieldCheck size={10}/>Always allow for this workflow</Button>
            <Button size="xs" variant="ghost" disabled={busy} onClick={() => void act(() => bridgeApi.resolveApproval(run.sessionId!, step.pending!.sequence, "decline"))}><X size={10}/>Decline</Button>
          </div>}
        </div>
      </div>)}</div>
    </div>
    <ErrorNote message={error}/>
    <p className="mt-3 text-caption text-muted-foreground">This run is read-only. Continue in chat to keep going by hand in a normal chat.</p>
  </div>;
}
