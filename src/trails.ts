/**
 * Pure helpers for the Trails screens: how summaries group, what a stage is
 * called, how a timeline splits into days, and what a brief selection costs.
 */

import type { TrailBriefItem, TrailEvent, TrailMemberRef, TrailState, TrailSummary } from "./protocol/generated/protocol";

/** One record the core's automatic sweep put on a trail. */
export type TrailMemberAdded = {
  trailId: string;
  trailTitle: string;
  member: TrailMemberRef;
  /** `PR #12 Fix login`, `Chat “Fix login”`. */
  label: string;
};

/** Starting a chat on a trail stopped before its first message was sent. */
export class TrailStartError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TrailStartError";
  }
}

/** The `trails/changed` notification. */
export type TrailsChangedPayload = {
  trailId?: string | null;
  added: TrailMemberAdded[];
};

export const TRAIL_STATE_ORDER: TrailState[] = ["needsYou", "inProgress", "shipped"];

export const TRAIL_STATE_LABEL: Record<TrailState, string> = {
  needsYou: "Needs you",
  inProgress: "In progress",
  shipped: "Shipped",
};

/** Stage 1 to 4 as the bar names them; 0 means nothing yet. */
export const TRAIL_STAGES = ["Plan", "Chats", "Pull request", "Shipped"] as const;

export function stageLabel(stage: number): string {
  return stage <= 0 ? "Not started" : TRAIL_STAGES[Math.min(stage, TRAIL_STAGES.length) - 1];
}

export type TrailSection = { state: TrailState; label: string; trails: TrailSummary[] };

/** Needs you, in progress, shipped; empty sections dropped; core order kept. */
export function groupTrails(trails: TrailSummary[]): TrailSection[] {
  return TRAIL_STATE_ORDER
    .map(state => ({ state, label: TRAIL_STATE_LABEL[state], trails: trails.filter(trail => trail.state === state && !trail.archivedAt) }))
    .filter(section => section.trails.length > 0);
}

export function needsYouCount(trails: TrailSummary[]): number {
  return trails.filter(trail => trail.state === "needsYou" && !trail.archivedAt).length;
}

/** The trail a chat sits on, if any. */
export function trailForSession(trails: TrailSummary[], sessionId: string | undefined | null): TrailSummary | undefined {
  if (!sessionId) return undefined;
  return trails.find(trail => trail.sessionIds.includes(sessionId));
}

function dayKey(at: string): string {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return "Undated";
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function dayLabel(at: string, now = new Date()): string {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return "Undated";
  const key = dayKey(at);
  if (key === dayKey(now.toISOString())) return "Today";
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (key === dayKey(yesterday.toISOString())) return "Yesterday";
  return date.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", ...(date.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }) });
}

export type TrailDay = { key: string; label: string; events: TrailEvent[] };

/** Events in the order given, grouped into consecutive local days. */
export function groupByDay(events: TrailEvent[], now = new Date()): TrailDay[] {
  const days: TrailDay[] = [];
  for (const event of events) {
    const key = dayKey(event.at);
    const last = days[days.length - 1];
    if (last && last.key === key) last.events.push(event);
    else days.push({ key, label: dayLabel(event.at, now), events: [event] });
  }
  return days;
}

export function timeLabel(at: string): string {
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
}

/** `2.9k`, `640`. */
export function formatTokens(tokens: number): string {
  if (tokens < 1000) return String(tokens);
  return `${(tokens / 1000).toFixed(tokens < 10_000 ? 1 : 0).replace(/\.0$/, "")}k`;
}

export function briefTotal(items: TrailBriefItem[], chosen: ReadonlySet<string>): number {
  return items.filter(item => chosen.has(item.id)).reduce((sum, item) => sum + item.tokens, 0);
}

export function defaultBriefSelection(items: TrailBriefItem[]): Set<string> {
  return new Set(items.filter(item => item.defaultOn).map(item => item.id));
}

/** Brief ids in the core's carry order (the preview's order). */
export function chosenInOrder(items: TrailBriefItem[], chosen: ReadonlySet<string>): string[] {
  return items.filter(item => chosen.has(item.id)).map(item => item.id);
}

/** What a member is, in words, for a remove button's accessible name. */
export function memberNoun(ref: TrailMemberRef): string {
  switch (ref.kind) {
    case "session": return "chat";
    case "note": return "note";
    case "pullRequest": return "pull request";
    case "outgoing": return "message";
    case "issue": return "issue";
  }
}
