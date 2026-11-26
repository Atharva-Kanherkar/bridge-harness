import { useState } from "react";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";
import { SkeletonRows } from "@/components/ui/skeleton";
import { Tip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import { relative } from "./UsageInsights";
import type { MemoryActivityEntry, MemoryDreamOperation, MemoryDreamReport, MemoryDreamTally } from "../types";

/** What a dream changed, in the order a person reads a night's cleanup. */
const DREAM_COUNTS: [keyof MemoryDreamTally, string][] = [
  ["merged", "merged"],
  ["corrected", "corrected"],
  ["expired", "set to expire"],
  ["grouped", "grouped"],
  ["retired", "retired"],
  ["approved", "approved"],
  ["rejected", "rejected"],
];

/** The local wall-clock time a dream settled, as 03:02. */
export function dreamClock(at: string): string {
  const instant = new Date(at);
  return `${String(instant.getHours()).padStart(2, "0")}:${String(instant.getMinutes()).padStart(2, "0")}`;
}

function spendLabel(microusd: number): string {
  const dollars = microusd / 1_000_000;
  return `$${dollars < 0.01 ? dollars.toFixed(4) : dollars.toFixed(2)}`;
}

/** One line for a settled dream: "Dreamed at 03:02: merged 6, approved 4 · $0.04". */
export function dreamSummary(entry: MemoryActivityEntry): string {
  const tally = entry.dream;
  const counts = tally ? DREAM_COUNTS.filter(([key]) => tally[key] > 0).map(([key, label]) => `${label} ${tally[key]}`) : [];
  const spend = (entry.spendMicrousd ?? 0) > 0 ? ` · ${spendLabel(entry.spendMicrousd ?? 0)}` : "";
  const undone = tally && tally.undone > 0 ? ` · ${tally.undone} undone` : "";
  return `Dreamed at ${dreamClock(entry.at)}: ${counts.length ? counts.join(", ") : "nothing to change"}${spend}${undone}`;
}

function changeLabel(operation: MemoryDreamOperation): string {
  switch (operation.operation) {
    case "merge": return `Merged ${operation.before.length}`;
    case "correct": return "Corrected";
    case "expire": return "Set to expire";
    case "group": return "Grouped";
    case "retire": return "Retired";
    case "approve": return "Approved";
    case "reject": return "Rejected";
    default: return operation.operation;
  }
}

/** One change: what it read, what it left, and its Undo. */
function DreamChange({ operation, pending, disabled, onUndo }: {
  operation: MemoryDreamOperation;
  pending: boolean;
  disabled: boolean;
  onUndo: () => void;
}) {
  const before = new Set(operation.before.map(record => record.id));
  const created = operation.after.filter(record => !before.has(record.id));
  const expiry = operation.operation === "expire" ? operation.after[0]?.expiresAt : undefined;
  return <li className="flex items-start gap-3 py-1.5">
    <div className="min-w-0 flex-1 space-y-0.5">
      <p className="text-foreground">
        {changeLabel(operation)}
        {operation.reason && <span className="text-faint"> · {operation.reason}</span>}
      </p>
      {operation.before.map(record => (
        <p key={record.id} className={cn("break-words text-faint", created.length > 0 && "line-through")}>{record.body}</p>
      ))}
      {created.map(record => <p key={record.id} className="break-words text-foreground">{record.body}</p>)}
      {expiry && <p className="text-faint">Expires {new Date(expiry).toLocaleDateString()}</p>}
    </div>
    {operation.undoneAt
      ? <span className="shrink-0 text-faint">Undone</span>
      : <Button variant="quiet" size="sm" loading={pending} disabled={disabled} onClick={onUndo}>Undo</Button>}
  </li>;
}

/** A settled dream in Recent runs. Opening it reads its report; every change
 *  has an Undo, and the night has one for all of them. */
export function DreamRun({ entry, onError }: { entry: MemoryActivityEntry; onError: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [report, setReport] = useState<MemoryDreamReport>();
  const [pending, setPending] = useState<number | "all" | null>(null);
  const runId = entry.runId;

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (!next || report || !runId) return;
    try {
      setReport(await bridgeApi.getDreamReport(runId));
    } catch (error) {
      setOpen(false);
      onError(friendlyError(error, "Couldn't open this dream."));
    }
  };

  const undo = async (index: number | "all") => {
    if (!runId) return;
    setPending(index);
    try {
      const result = index === "all" ? await bridgeApi.undoDream(runId) : await bridgeApi.undoDreamOperation(runId, index);
      setReport(result.report);
      if (result.skipped.length > 0) {
        const plural = result.skipped.length === 1 ? "change was" : "changes were";
        onError(`${result.skipped.length} ${plural} left as they are. ${result.skipped[0].reason}`);
      }
    } catch (error) {
      onError(friendlyError(error, "Couldn't undo this change."));
    } finally {
      setPending(null);
    }
  };

  const summary = dreamSummary(entry);
  const allUndone = report?.operations.every(operation => operation.undoneAt) ?? true;
  return <li className="py-2 text-caption">
    <RowButton size="bare" shape="flush" className="w-full text-left" aria-expanded={open} disabled={!runId} onClick={() => void toggle()}>
      <span className="flex w-full items-baseline gap-3">
        <span className="w-24 shrink-0 font-mono text-caption text-faint font-medium">dream</span>
        <Tip label={summary}><span className="min-w-0 flex-1 truncate text-foreground">{summary}</span></Tip>
        <span className="shrink-0 tabular-nums text-faint">{relative(entry.at)}</span>
      </span>
    </RowButton>
    {open && <div aria-label="Dream changes" className="mt-2 border-l border-border pl-3">
      {report === undefined
        ? <SkeletonRows label="Reading this dream" rows={2} meta={false} framed={false} />
        : report.operations.length === 0
          ? <p className="py-1 text-faint">Nothing changed this night.</p>
          : <>
            <ul className="divide-y divide-border">
              {report.operations.map(operation => (
                <DreamChange
                  key={operation.index}
                  operation={operation}
                  pending={pending === operation.index}
                  disabled={pending !== null}
                  onUndo={() => void undo(operation.index)}
                />
              ))}
            </ul>
            <Button className="mt-1" variant="quiet" size="sm" loading={pending === "all"} disabled={pending !== null || allUndone} onClick={() => void undo("all")}>Undo all</Button>
          </>}
    </div>}
  </li>;
}
