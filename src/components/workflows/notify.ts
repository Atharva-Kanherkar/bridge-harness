import type { WorkflowRun } from "../../protocol/generated/protocol";

export type WorkflowNotice = { title: string; body: string };

/**
 * Whether a run's new status deserves a native notification, and what it says.
 *
 * Each run and status notifies once (`seen` remembers what was said). Anything
 * that needs the person, or went wrong, always notifies: waiting for approval,
 * waiting for quota, failed, and unverified (it ended without the result it was
 * asked for). A success notifies only when its workflow asked for that.
 */
export function workflowNotification(run: WorkflowRun, name: string, notifyOnSuccess: boolean, seen: Set<string>): WorkflowNotice | null {
  const key = `${run.id}:${run.status}`;
  if (seen.has(key)) return null;
  let notice: WorkflowNotice | null = null;
  switch (run.status) {
    case "waiting_approval":
      notice = { title: `${name} needs you`, body: "It wants to change something. Allow or decline it in Workflows." };
      break;
    case "waiting_quota":
      notice = { title: `${name} is waiting`, body: "Your plan is near its limit, so it will run when there is room." };
      break;
    case "failed":
      notice = { title: `${name} failed`, body: run.summary ?? run.failureCode ?? "Open the run to see why." };
      break;
    case "unverified":
      notice = { title: `${name} needs a look`, body: run.summary ?? "It finished without the result it was asked for." };
      break;
    case "succeeded":
      notice = notifyOnSuccess ? { title: `${name} finished`, body: run.summary ?? "The run succeeded." } : null;
      break;
    default:
      notice = null;
  }
  if (notice) seen.add(key);
  return notice;
}

/** What the sidebar counts: runs waiting for the person, and unread results. */
export function needsYouCount(runs: WorkflowRun[]): number {
  return runs.filter(run => run.status === "waiting_approval" || (run.unread && run.status !== "running" && run.status !== "queued")).length;
}
