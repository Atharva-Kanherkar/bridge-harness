import type { WorkflowEventKind, WorkflowRunStatus, WorkflowTrigger } from "../../protocol/generated/protocol";

const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const pad = (value: number) => String(value).padStart(2, "0");

/** The events a workflow can start on, as a person would say them. */
export type EventApp = "Slack" | "GitHub" | "Gmail" | "Linear" | "Notion";
export const EVENT_APPS: EventApp[] = ["Slack", "GitHub", "Gmail", "Linear", "Notion"];

export const EVENT_OPTIONS: Array<{ value: WorkflowEventKind; app: EventApp; label: string }> = [
  { value: "slack_mention", app: "Slack", label: "I'm mentioned in Slack" },
  { value: "slack_direct_message", app: "Slack", label: "I get a Slack direct message" },
  { value: "slack_thread_reply", app: "Slack", label: "Someone replies in my Slack thread" },
  { value: "github_pull_request_opened", app: "GitHub", label: "A pull request is opened" },
  { value: "github_issue_opened", app: "GitHub", label: "An issue is opened" },
  { value: "github_review_requested", app: "GitHub", label: "My review is requested on GitHub" },
  { value: "gmail_message_received", app: "Gmail", label: "A new email arrives" },
  { value: "linear_issue_assigned", app: "Linear", label: "An issue is assigned to me in Linear" },
  { value: "notion_page_created", app: "Notion", label: "A page is added to a Notion database" },
];

export const isGithubEvent = (event: WorkflowEventKind) => event.startsWith("github_");

/** The filters an event takes, as its fields' labels and placeholders. `required` marks one that must be set. */
export type EventFilter = { label: string; placeholder: string; required?: boolean };
export function eventFilters(event: WorkflowEventKind): { repo?: EventFilter; channel?: EventFilter; from?: EventFilter } {
  if (isGithubEvent(event)) return { repo: { label: "Repository", placeholder: "owner/name", required: needsRepo(event) } };
  switch (event) {
    case "gmail_message_received": return { from: { label: "From", placeholder: "a name or address" } };
    case "linear_issue_assigned": return { channel: { label: "Team", placeholder: "Platform" } };
    case "notion_page_created": return { channel: { label: "Database", placeholder: "Roadmap, or a link", required: true }, from: { label: "Created by", placeholder: "a name" } };
    default: return { channel: { label: "In channel", placeholder: "#eng" }, from: { label: "From", placeholder: "a name" } };
  }
}

export const needsRepo = (event: WorkflowEventKind) => event === "github_pull_request_opened" || event === "github_issue_opened";

/** Plain words for an event trigger: "When I'm mentioned in Slack in #eng". */
export function eventSummary(trigger: Extract<WorkflowTrigger, { kind: "event" }>): string {
  const option = EVENT_OPTIONS.find(candidate => candidate.value === trigger.event);
  // The dropdown's labels start with a capital; mid-sentence they do not,
  // except "I" ("When I'm mentioned").
  const label = option?.label ?? trigger.event;
  const phrase = /^I\b|^I'/.test(label) ? label : label.charAt(0).toLowerCase() + label.slice(1);
  let text = `When ${phrase}`;
  if (trigger.event === "notion_page_created") {
    text = `When a page is added to ${trigger.channel ? `${trigger.channel} in Notion` : "a Notion database"}`;
    if (trigger.from) text += ` by ${trigger.from}`;
  } else if (trigger.event === "linear_issue_assigned") {
    if (trigger.channel) text += ` (team ${trigger.channel})`;
  } else {
    if (trigger.repo) text += ` in ${trigger.repo}`;
    if (trigger.channel) text += ` in ${trigger.channel.startsWith("#") ? trigger.channel : `#${trigger.channel}`}`;
    if (trigger.from) text += ` from ${trigger.from}`;
  }
  if (trigger.batch) text += `, collected and run ${cronSummary(trigger.batch.cron).replace(/^Every day/, "every day").replace(/^Weekdays/, "on weekdays")}`;
  return text;
}

/** Plain words for a trigger. Unknown cron shapes are shown as written. */
export function triggerSummary(trigger: WorkflowTrigger): string {
  if (trigger.kind === "manual") return "On demand";
  if (trigger.kind === "event") return eventSummary(trigger);
  return cronSummary(trigger.cron);
}

/** Plain words for a 5-field cron expression. */
export function cronSummary(cron: string): string {
  const [minute, hour, dom, month, dow] = cron.trim().split(/\s+/);
  const time = /^\d+$/.test(minute) && /^\d+$/.test(hour) ? `${pad(Number(hour))}:${pad(Number(minute))}` : null;
  if (time && dom === "*" && month === "*") {
    if (dow === "*") return `Every day at ${time}`;
    if (dow === "1-5") return `Weekdays at ${time}`;
    if (/^[0-7]$/.test(dow)) return `${DAY_NAMES[Number(dow) % 7]}s at ${time}`;
  }
  if (time && /^\d+$/.test(dom) && month === "*" && dow === "*") return `Monthly on day ${dom} at ${time}`;
  if (/^\d+$/.test(minute) && hour === "*" && dom === "*" && month === "*" && dow === "*") return `Every hour at :${pad(Number(minute))}`;
  const every = /^\*\/(\d+)$/.exec(minute);
  if (every && hour === "*" && dom === "*" && month === "*" && dow === "*") return `Every ${every[1]} minutes`;
  return `Cron ${cron}`;
}

/** "in 14h", "2h ago", or null. */
export function relativeTime(iso: string | null | undefined, now: number): string | null {
  if (!iso) return null;
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return null;
  const delta = at - now;
  const magnitude = Math.abs(delta);
  const [unit, label] = magnitude < 3_600_000 ? [60_000, "m"] : magnitude < 86_400_000 ? [3_600_000, "h"] : [86_400_000, "d"];
  const amount = Math.max(1, Math.round(magnitude / unit));
  return delta >= 0 ? `in ${amount}${label}` : `${amount}${label} ago`;
}

export const RUN_STATUS_LABEL: Record<WorkflowRunStatus, string> = {
  queued: "queued",
  waiting_quota: "waiting for quota",
  running: "running",
  waiting_approval: "waiting for you",
  succeeded: "succeeded",
  unverified: "unverified",
  failed: "failed",
  cancelled: "cancelled",
  skipped: "skipped",
};

export type Tone = "ok" | "warn" | "bad" | "idle";

export function runTone(status: WorkflowRunStatus): Tone {
  if (status === "succeeded") return "ok";
  if (status === "failed") return "bad";
  if (status === "waiting_approval" || status === "waiting_quota") return "warn";
  return "idle";
}

export const ACTIVE_RUN_STATUSES: WorkflowRunStatus[] = ["queued", "waiting_quota", "running", "waiting_approval"];

/** Schedule presets the editor offers; each compiles to 5-field cron. */
export type SchedulePreset = "manual" | "event" | "daily" | "weekdays" | "weekly" | "hourly" | "custom";

export function presetCron(preset: SchedulePreset, time: string, weekday: number, custom: string): string | null {
  const [hour, minute] = time.split(":").map(Number);
  const safeHour = Number.isFinite(hour) ? hour : 9;
  const safeMinute = Number.isFinite(minute) ? minute : 0;
  switch (preset) {
    case "manual":
    case "event": return null;
    case "daily": return `${safeMinute} ${safeHour} * * *`;
    case "weekdays": return `${safeMinute} ${safeHour} * * 1-5`;
    case "weekly": return `${safeMinute} ${safeHour} * * ${weekday}`;
    case "hourly": return `${safeMinute} * * * *`;
    case "custom": return custom.trim();
  }
}

/** The editor's starting point for an existing trigger. */
export function presetFor(trigger: WorkflowTrigger): { preset: SchedulePreset; time: string; weekday: number; custom: string } {
  if (trigger.kind === "manual") return { preset: "manual", time: "09:00", weekday: 1, custom: "" };
  if (trigger.kind === "event") return { ...presetForCron(trigger.batch?.cron ?? "0 9 * * *"), preset: "event" };
  return presetForCron(trigger.cron);
}

export function presetForCron(cron: string): { preset: SchedulePreset; time: string; weekday: number; custom: string } {
  const trigger = { cron };
  const [minute, hour, dom, month, dow] = trigger.cron.trim().split(/\s+/);
  const numeric = /^\d+$/.test(minute) && /^\d+$/.test(hour);
  const time = numeric ? `${pad(Number(hour))}:${pad(Number(minute))}` : "09:00";
  if (numeric && dom === "*" && month === "*") {
    if (dow === "*") return { preset: "daily", time, weekday: 1, custom: trigger.cron };
    if (dow === "1-5") return { preset: "weekdays", time, weekday: 1, custom: trigger.cron };
    if (/^[0-7]$/.test(dow)) return { preset: "weekly", time, weekday: Number(dow) % 7, custom: trigger.cron };
  }
  if (/^\d+$/.test(minute) && hour === "*" && dom === "*" && month === "*" && dow === "*") {
    return { preset: "hourly", time: `00:${pad(Number(minute))}`, weekday: 1, custom: trigger.cron };
  }
  return { preset: "custom", time: "09:00", weekday: 1, custom: trigger.cron };
}

export const WEEKDAY_OPTIONS = DAY_NAMES.map((name, index) => ({ value: index, label: name }));

export function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** "$0.012", "under $0.01", "$1.20". Bridge's figures are estimates at API prices. */
export function formatCost(microusd: number | null | undefined): string | null {
  if (microusd == null) return null;
  const dollars = microusd / 1_000_000;
  if (dollars === 0) return "$0";
  if (dollars < 0.01) return "under $0.01";
  return dollars < 1 ? `$${dollars.toFixed(dollars < 0.1 ? 3 : 2)}` : `$${dollars.toFixed(2)}`;
}

/** "≈ $0.04 a run · ≈ $0.88 a month", or null without data. */
export function costLine(average: number | null | undefined, monthly: number | null | undefined): string | null {
  const run = formatCost(average);
  if (!run) return null;
  const month = formatCost(monthly);
  return month ? `≈ ${run} a run · ≈ ${month} a month` : `≈ ${run} a run`;
}

/** "1st", "2nd", "3rd", "4th". */
export function ordinal(position: number): string {
  const tens = position % 100;
  if (tens >= 11 && tens <= 13) return `${position}th`;
  return `${position}${["th", "st", "nd", "rd"][position % 10] ?? "th"}`;
}
