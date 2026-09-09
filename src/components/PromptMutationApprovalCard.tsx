import { useRef, useState } from "react";
import { Check, X } from "lucide-react";
import type { ConversationItem } from "../conversation";
import type { ApprovalDecision } from "../types";
import type { InteractionResolutionResult } from "../protocol/generated/protocol";

const BUTTON = "inline-flex min-h-8 items-center gap-1.5 rounded-lg px-3 py-1 text-[13px] font-medium transition-colors disabled:opacity-50";

function text(data: Record<string, unknown>, key: string): string {
  return typeof data[key] === "string" ? data[key] : "";
}

function outcomeCopy(status: string): string {
  if (status === "accept" || status === "accepted" || status === "allowed_once") return "Approved. Guidance saved for the next start or relaunch.";
  if (status === "stale" || status === "conflict") return "This proposal is out of date. No guidance was changed; a new proposal is needed.";
  if (status === "decline" || status === "declined") return "Declined. No guidance was changed.";
  if (status === "cancel" || status === "cancelled") return "Cancelled. No guidance was changed.";
  if (status === "failed") return "The prompt change could not be applied.";
  if (status === "denied") return "This proposal is no longer authorized. No guidance was changed.";
  return "This proposal has been resolved.";
}

/** Host-authored snapshots are shown literally: rendering either prompt as
 * Markdown would let the proposed instructions disguise the bytes reviewed. */
export function PromptMutationApprovalCard({ item, onResolve }: {
  item: ConversationItem;
  onResolve: (eventId: number, decision: ApprovalDecision, optionId?: string) => Promise<InteractionResolutionResult | void> | void;
}) {
  const [busy, setBusy] = useState<"accept" | "decline" | null>(null);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<InteractionResolutionResult>();
  const resolving = useRef(false);
  const status = result?.decision ?? item.status ?? "pending";
  const pending = status === "pending";
  const accepted = status === "accept" || status === "accepted" || status === "allowed_once";
  const target = text(item.data, "target");
  const role = target.startsWith("worker:") ? target.slice("worker:".length) : target;
  const resolution = item.data.resolution && typeof item.data.resolution === "object"
    ? item.data.resolution as Record<string, unknown>
    : {};
  const resolutionData = resolution.data && typeof resolution.data === "object"
    ? resolution.data as Record<string, unknown>
    : {};
  const resolutionReason = result?.reason ?? (text(resolution, "reason") || text(resolutionData, "reason"));

  async function resolve(decision: "accept" | "decline") {
    if (resolving.current || !pending) return;
    resolving.current = true;
    setBusy(decision);
    setError(undefined);
    try {
      const resolved = await onResolve(item.eventId, decision);
      if (resolved) setResult(resolved);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      resolving.current = false;
      setBusy(null);
    }
  }

  return <section aria-label="Review prompt change" className="min-w-0 overflow-hidden rounded-lg border border-border border-l-2 border-l-warning bg-card">
    <header className="flex flex-wrap items-baseline gap-2 px-3.5 pt-3 sm:px-4">
      <b className="text-[13px] font-semibold text-foreground">Review prompt change</b>
      {pending && <small className="text-[11px] text-warning">waiting for you</small>}
    </header>
    <div className="space-y-3 px-3.5 pt-2 text-[13px] sm:px-4">
      <p className="text-foreground">Append to the <b>{role}</b> role default · Shared role guidance</p>
      <p className="leading-relaxed text-muted-foreground">Shared by all sessions with this role. Applies the next time Bridge starts or relaunches a matching role. Running turns keep their current instructions.</p>
      <p className="break-words text-[12px] text-muted-foreground">Requested by {text(item.data, "actorRole")} · session <code className="font-mono">{text(item.data, "actorSessionId")}</code></p>
      <div>
        <span className="font-medium text-foreground">Reason for the change</span>
        <p className="mt-1 whitespace-pre-wrap break-words leading-relaxed text-muted-foreground">{text(item.data, "rationale")}</p>
      </div>
      <div className="grid min-w-0 gap-3 md:grid-cols-2">
        {(["Before", "After"] as const).map(label => {
          const content = text(item.data, label === "Before" ? "beforeText" : "afterText");
          return <div key={label} className="min-w-0">
            <span className="text-[12px] font-medium text-foreground">{label}{content === "" && <span className="font-normal text-muted-foreground"> · empty guidance</span>}</span>
            <pre aria-label={`${label} prompt text`} className="mt-1 max-h-72 min-h-16 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[12px] leading-relaxed text-foreground">{content}</pre>
          </div>;
        })}
      </div>
    </div>
    {pending ? <div className="flex flex-wrap justify-end gap-2 px-3.5 py-3 sm:px-4" aria-busy={busy !== null}>
      <button type="button" disabled={busy !== null} className={`${BUTTON} border border-input text-foreground hover:bg-accent`} onClick={() => void resolve("decline")}><X size={12} aria-hidden="true" />{busy === "decline" ? "Declining…" : "Decline"}</button>
      <button type="button" disabled={busy !== null} className={`${BUTTON} bg-primary text-primary-foreground hover:bg-primary/90`} onClick={() => void resolve("accept")}><Check size={12} aria-hidden="true" />{busy === "accept" ? "Approving…" : "Approve change"}</button>
    </div> : <div role="status" className="space-y-1 px-3.5 py-3 text-[12px] text-muted-foreground sm:px-4">
      <p className="flex items-start gap-1.5">{accepted ? <Check size={12} className="mt-0.5 shrink-0" aria-hidden="true" /> : <X size={12} className="mt-0.5 shrink-0" aria-hidden="true" />}{outcomeCopy(status)}</p>
      {result?.disposition === "alreadyResolved" && <p>This request was already resolved.</p>}
      {resolutionReason && <p className="whitespace-pre-wrap break-words">{resolutionReason}</p>}
      {accepted && <p>Review history or restore an earlier revision in Settings → Prompts.</p>}
    </div>}
    {error && <p role="alert" className="px-3.5 pb-3 text-[12px] text-destructive sm:px-4">{error}</p>}
  </section>;
}
