import type { WorkTask, WorkTaskState } from "../protocol/generated/protocol";

// How a suggested task reads, and which buttons it gets.
//
// Facts and tasks look similar on the board and are trusted very differently. A fact is a
// projection of Bridge's own state; a task is a model's summary of something it read
// somewhere else. So a task row always says where it came from and how confident the model
// was — not because the number is precise, but because a claim with a source attached
// invites the reader to check it, and one without does not.

/** Which actions a task in this state offers. Mirrors
 * `bridge_core::work_task_state::apply_action`, which is the enforcing side: this decides
 * what to *show*, and a button the backend would refuse must never appear. */
export const ACTIONS_BY_STATE: Record<WorkTaskState, TaskAction[]> = {
  active: ["start", "done", "snooze", "dismiss"],
  stale: ["start", "done", "snooze", "dismiss"],
  snoozed: ["restore", "done", "dismiss"],
  done: [],
  dismissed: ["restore"],
};

export type TaskAction = "start" | "done" | "snooze" | "dismiss" | "restore";

export const ACTION_LABEL: Record<TaskAction, string> = {
  start: "Start",
  done: "Done",
  snooze: "Snooze 1 day",
  dismiss: "Dismiss",
  restore: "Restore",
};

/** Which tasks a reader sees. Mirrors `TaskState::visible`: a stale task is hidden unless
 * it is pinned, because pinning is how a user says keep this in front of me. */
export function isVisibleTask(task: Pick<WorkTask, "state" | "pinned">): boolean {
  if (task.state === "active") return true;
  if (task.state === "stale") return task.pinned;
  return false;
}

/** Visible tasks, pinned first and then by rank.
 *
 * Pinned-first is the one place the client reorders, and it is reordering by a decision the
 * user made rather than second-guessing the model's ranking. */
export function orderTasks(tasks: WorkTask[]): WorkTask[] {
  return tasks
    .filter(isVisibleTask)
    .slice()
    .sort((left, right) => {
      if (left.pinned !== right.pinned) return left.pinned ? -1 : 1;
      if (left.rank !== right.rank) return left.rank - right.rank;
      return left.id.localeCompare(right.id);
    });
}

/** Confidence as a word rather than a number.
 *
 * A model's 82% is not a measurement, and printing it to the percent invites a precision it
 * does not have. Three bands say what the number is worth. */
export function confidenceLabel(confidenceBps: number): string {
  if (confidenceBps >= 7_500) return "high confidence";
  if (confidenceBps >= 4_000) return "medium confidence";
  return "low confidence";
}

/** Where a task came from, in words: the family, and the account when there is one. */
export function sourceLabel(task: Pick<WorkTask, "sourceKind" | "connectorInstanceId">): string {
  const family = task.sourceKind.split(".")[0] || task.sourceKind;
  const pretty = family.charAt(0).toUpperCase() + family.slice(1);
  return task.connectorInstanceId ? `${pretty} · ${task.connectorInstanceId}` : pretty;
}

/** Whether this task's evidence can be opened at all.
 *
 * Only a shape the backend produced. The real check happens in Rust when the target is
 * opened — this decides whether to draw the affordance, and drawing one that would be
 * refused is worse than drawing none. */
export function hasOpenableEvidence(task: Pick<WorkTask, "evidenceTarget">): boolean {
  const target = task.evidenceTarget;
  if (!target) return false;
  if (target.kind === "session") return Boolean(target.sessionId);
  return target.kind === "externalLink" && target.url.startsWith("https://");
}

/** The label for opening this task's evidence, which names where it goes. */
export function evidenceLabel(task: Pick<WorkTask, "evidenceTarget">): string | null {
  const target = task.evidenceTarget;
  if (!target) return null;
  if (target.kind === "session") return "Open session";
  return target.kind === "externalLink" ? `Open on ${target.host}` : null;
}

/** What a screen reader hears for a task row. Source, confidence and state as words, since
 * none of the three is carried by colour. */
export function taskAnnouncement(task: WorkTask): string {
  const state = task.state === "stale" ? ", stale" : task.pinned ? ", pinned" : "";
  return `Suggested from ${sourceLabel(task)}${state}: ${task.title}. ${confidenceLabel(task.confidenceBps)}.`;
}
