import type { WorkflowRunStatus, WorkflowTrigger } from "../../protocol/generated/protocol";

const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const pad = (value: number) => String(value).padStart(2, "0");

/** Plain words for a trigger. Unknown cron shapes are shown as written. */
export function triggerSummary(trigger: WorkflowTrigger): string {
  if (trigger.kind === "manual") return "On demand";
  const [minute, hour, dom, month, dow] = trigger.cron.trim().split(/\s+/);
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
  return `Cron ${trigger.cron}`;
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
export type SchedulePreset = "manual" | "daily" | "weekdays" | "weekly" | "hourly" | "custom";

export function presetCron(preset: SchedulePreset, time: string, weekday: number, custom: string): string | null {
  const [hour, minute] = time.split(":").map(Number);
  const safeHour = Number.isFinite(hour) ? hour : 9;
  const safeMinute = Number.isFinite(minute) ? minute : 0;
  switch (preset) {
    case "manual": return null;
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
