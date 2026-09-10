import type { WorkTask } from "../protocol/generated/protocol";

const DAY_MS = 24 * 60 * 60 * 1000;
const SOURCES = new Set(["slack.message", "github.item", "gmail.thread", "linear.issue", "notion.page"]);

/** A rolling source-activity window, independent of cache reads or local task edits. */
export function recentIntegrationActivity(tasks: WorkTask[], now: Date): WorkTask[] {
  const end = now.getTime();
  return tasks.filter(task => {
    const activity = task.sourceActivityAt ? Date.parse(task.sourceActivityAt) : NaN;
    return SOURCES.has(task.sourceKind) && Boolean(task.connectorInstanceId)
      && task.state === "active"
      && activity >= end - DAY_MS && activity <= end;
  }).sort((a, b) => Date.parse(b.sourceActivityAt!) - Date.parse(a.sourceActivityAt!) || a.id.localeCompare(b.id));
}
