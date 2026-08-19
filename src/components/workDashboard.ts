// The dashboard header's words, kept out of the component so every phrasing is
// testable with a fixed clock. Nothing here touches a network or a store: it
// summarises the board the backend already produced.
//
// Two rules the copy holds to: no secrets and no raw provider errors — a failed
// run is named by its stable code, never its detail text — and a source that
// was not read is never described as read.

import type { WorkBriefRun, WorkSourceCoverage } from "../protocol/generated/protocol";

export const FAMILY_LABEL: Record<string, string> = {
  slack: "Slack",
  gmail: "Gmail",
  github: "GitHub",
  linear: "Linear",
  notion: "Notion",
};

function familyLabel(row: WorkSourceCoverage): string {
  return FAMILY_LABEL[row.connectorFamily] ?? row.connectorInstanceId;
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}

/** Which tools the last run read, could not reach, or needs signed in — from
 * the run's own coverage, never from the model's word. */
export function toolsReadLine(sources: WorkSourceCoverage[]): string {
  if (sources.length === 0) return "No tools were read.";
  const read = unique(sources.filter(row => row.status === "succeeded").map(familyLabel));
  const failed = unique(sources.filter(row => row.status === "failed").map(familyLabel));
  const needsAuth = unique(sources.filter(row => row.status === "auth_required").map(familyLabel));
  const parts: string[] = [];
  parts.push(read.length > 0 ? `Read ${read.join(", ")}` : "Nothing was read");
  if (failed.length > 0) parts.push(`${failed.join(", ")} could not be reached`);
  if (needsAuth.length > 0) {
    parts.push(`${needsAuth.join(", ")} needs sign-in — reconnect it in your harness`);
  }
  return `${parts.join(" · ")}.`;
}

export function relativeTime(iso: string, now: Date): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "at an unknown time";
  const seconds = Math.max(0, Math.floor((now.getTime() - then) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** How the last run ended. The failure code is a stable vocabulary; the detail
 * column is deliberately not surfaced here. */
export function lastRunLine(run: WorkBriefRun | null | undefined, now: Date): string {
  if (!run) return "No briefing has run yet.";
  switch (run.status) {
    case "running":
      return "A briefing is running now.";
    case "succeeded":
      return `Last briefing ${relativeTime(run.completedAt ?? run.startedAt, now)} · completed.`;
    case "cancelled":
      return `Last briefing ${relativeTime(run.completedAt ?? run.startedAt, now)} · cancelled.`;
    case "skipped":
      return `Last briefing ${relativeTime(run.startedAt, now)} · skipped.`;
    default:
      return `Last briefing ${relativeTime(run.completedAt ?? run.startedAt, now)} · failed (${run.failureCode ?? "unknown"}). The previous board is untouched.`;
  }
}
