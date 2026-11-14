import type { SessionPullRequest } from "./protocol/generated/protocol";
import { chatPrStatus, checkTone, type ChatPrPhase } from "./chatPullRequests";
import type { BackgroundTaskView } from "./backgroundTasks";

// The words behind the composer shelf and the Work pane: one phrase per PR,
// the counts when there are several, and the task summary. Pure, so every
// state the shelf can be in is assertable without a DOM, and the shelf and the
// pane can never word the same PR two ways.

/** Ink for a phrase. Running stays neutral: only an outcome earns color. */
export type ShelfTone = "danger" | "success" | "info" | "neutral" | "faint";

export type ShelfPhrase = { text: string; tone: ShelfTone };

const RANK: Record<ChatPrPhase, number> = {
  conflicts: 0, failing: 1, running: 2, passing: 3, none: 4, closed: 5, merged: 6,
};

/** Worst first — what needs the person leads — and stable within a phase. */
export function orderPullRequests(prs: readonly SessionPullRequest[]): SessionPullRequest[] {
  return prs
    .map((pr, index) => ({ pr, index, rank: RANK[chatPrStatus(pr).phase] }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map(({ pr }) => pr);
}

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

export function failingChecks(pr: SessionPullRequest) {
  return pr.checkDetails.filter(check => checkTone(check) === "failed");
}

/** The status half of one shelf line or pane row. */
export function shelfPhrase(pr: SessionPullRequest): ShelfPhrase {
  const { checks } = pr;
  const phase = chatPrStatus(pr).phase;
  const draft = pr.isDraft && pr.state === "open" ? "Draft · " : "";
  switch (phase) {
    case "merged": return { text: "Merged", tone: "info" };
    case "closed": return { text: "Closed", tone: "faint" };
    case "conflicts": return { text: `${draft}Merge conflicts`, tone: "danger" };
    case "failing": {
      const failed = failingChecks(pr);
      const text = checks.failed === 1 && failed.length === 1 ? `${failed[0].name} failed` : `${plural(checks.failed, "check")} failed`;
      return { text: draft + text, tone: "danger" };
    }
    case "running": {
      if (checks.inProgress === 0) return { text: `${draft}${checks.queued} queued`, tone: "neutral" };
      const settled = checks.total - checks.queued - checks.inProgress;
      const running = pr.checkDetails.filter(check => checkTone(check) === "running");
      const tail = checks.inProgress === 1 && running.length === 1 ? `${running[0].name} running` : "running";
      return { text: `${draft}${settled} of ${checks.total} · ${tail}`, tone: "neutral" };
    }
    case "passing": return { text: `${draft}Checks passed`, tone: "success" };
    case "none": return { text: `${draft}${checks.total > 0 ? "Checks skipped" : "No checks"}`, tone: "faint" };
  }
}

export type CountPart = { count: number; label: string; tone: ShelfTone };

/** The several-PR line: what each count means, worst first, zeros dropped. */
export function prCounts(prs: readonly SessionPullRequest[]): CountPart[] {
  const tally = { conflicts: 0, failing: 0, running: 0, passing: 0, merged: 0, closed: 0, none: 0 };
  for (const pr of prs) tally[chatPrStatus(pr).phase] += 1;
  const parts: CountPart[] = [
    { count: tally.conflicts, label: "conflicting", tone: "danger" },
    { count: tally.failing, label: "failing", tone: "danger" },
    { count: tally.running, label: "running", tone: "neutral" },
    { count: tally.passing, label: "passed", tone: "success" },
    { count: tally.merged, label: "merged", tone: "info" },
    { count: tally.closed, label: "closed", tone: "faint" },
  ];
  return parts.filter(part => part.count > 0);
}

/** The sidebar row's chip: the most urgent PR in a word or two, so a long
 * chat list shows at a glance which chats landed and which need a hand. */
export type SidebarPrChip = {
  pr: SessionPullRequest;
  text: string;
  tone: ShelfTone;
  /** PRs beyond the one shown. */
  more: number;
  /** Every PR, one per line, for the hover. */
  title: string;
};

function sidebarWord(pr: SessionPullRequest): { text: string; tone: ShelfTone } {
  const status = chatPrStatus(pr);
  switch (status.phase) {
    case "merged": return { text: "merged", tone: "info" };
    case "closed": return { text: "closed", tone: "faint" };
    case "conflicts": return { text: "conflicts", tone: "danger" };
    case "failing": return { text: `${pr.checks.failed} failing`, tone: "danger" };
    case "running": return { text: "CI running", tone: "neutral" };
    case "passing": return { text: pr.isDraft ? "draft" : "passing", tone: pr.isDraft ? "faint" : "success" };
    case "none": return { text: pr.isDraft ? "draft" : "open", tone: "faint" };
  }
}

export function sidebarPrChip(prs: readonly SessionPullRequest[] | undefined): SidebarPrChip | null {
  if (!prs?.length) return null;
  const ordered = orderPullRequests(prs);
  const lead = ordered[0];
  const title = ordered
    .map(pr => `PR #${pr.number} · ${chatPrStatus(pr).headline}${pr.stale ? " (last known)" : ""}`)
    .join("\n");
  return { pr: lead, ...sidebarWord(lead), more: ordered.length - 1, title };
}

export type TaskSummary =
  | { kind: "none" }
  | { kind: "one"; task: BackgroundTaskView }
  | { kind: "many"; running: number; watching: number; text: string; oldestMs?: number };

export function taskSummary(tasks: readonly BackgroundTaskView[]): TaskSummary {
  if (tasks.length === 0) return { kind: "none" };
  if (tasks.length === 1) return { kind: "one", task: tasks[0] };
  const running = tasks.filter(task => task.kind === "running").length;
  const watching = tasks.length - running;
  const text = [running && `${running} running`, watching && `${watching} watching`].filter(Boolean).join(" · ");
  const elapsed = tasks.map(task => task.elapsedMs).filter((value): value is number => value !== undefined);
  return { kind: "many", running, watching, text, oldestMs: elapsed.length ? Math.max(...elapsed) : undefined };
}

/**
 * A draft for the composer, never a send: "Fix it" proposes the next message
 * and the person decides. Names every failing check, with its log when GitHub
 * reported one, so the agent starts from the evidence rather than a guess.
 */
export function fixPrompt(pr: SessionPullRequest): string {
  const failed = failingChecks(pr);
  if (failed.length === 0) return `CI is failing on #${pr.number}. Find the failing check, read its log, and fix the cause.`;
  const lines = failed.map(check => `- ${check.name}${check.logUrl ? ` (log: ${check.logUrl})` : ""}`);
  const lead = failed.length === 1 ? "This check is" : "These checks are";
  return [`${lead} failing on #${pr.number}:`, ...lines, "Read the failing log, find the cause, and push a fix."].join("\n");
}

/** The shelf's accessible name: the whole line, in words. */
export function shelfLabel(prs: readonly SessionPullRequest[], tasks: TaskSummary): string {
  const parts: string[] = [];
  if (prs.length === 1) {
    const pr = prs[0];
    parts.push(`Pull request #${pr.number}: ${shelfPhrase(pr).text}${pr.stale ? ", last known state" : ""}`);
  } else if (prs.length > 1) {
    parts.push(`${prs.length} pull requests: ${prCounts(prs).map(part => `${part.count} ${part.label}`).join(", ")}`);
  }
  if (tasks.kind === "one") parts.push(`In the background: ${tasks.task.label}`);
  if (tasks.kind === "many") parts.push(`In the background: ${tasks.text}`);
  return `${parts.join(". ")}. Open the Work pane.`;
}
