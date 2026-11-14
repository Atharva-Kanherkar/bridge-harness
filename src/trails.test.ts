import { describe, expect, it } from "vitest";
import type { TrailBriefItem, TrailEvent, TrailSummary } from "./protocol/generated/protocol";
import { briefTotal, chosenInOrder, dayLabel, defaultBriefSelection, formatTokens, groupByDay, groupTrails, needsYouCount, stageLabel, trailForSession } from "./trails";

const trail = (id: string, state: TrailSummary["state"], extra: Partial<TrailSummary> = {}): TrailSummary => ({
  id, title: id, state, stage: 2, harnesses: [], sessionIds: [], memberCount: 0, createdAt: "2026-11-10T00:00:00Z", updatedAt: "2026-11-10T00:00:00Z", ...extra,
});

const event = (id: string, at: string): TrailEvent => ({
  id, kind: "chat", at, title: id, tone: "plain", member: { kind: "session", ref: id }, addedBy: "person",
});

describe("trails helpers", () => {
  it("groups needs you, in progress and shipped, drops empty sections and archived trails, keeps order", () => {
    const sections = groupTrails([trail("a", "shipped"), trail("b", "needsYou"), trail("c", "needsYou"), trail("d", "inProgress", { archivedAt: "x" })]);
    expect(sections.map(section => [section.label, section.trails.map(item => item.id)])).toEqual([
      ["Needs you", ["b", "c"]],
      ["Shipped", ["a"]],
    ]);
    expect(needsYouCount([trail("a", "needsYou"), trail("b", "needsYou", { archivedAt: "x" }), trail("c", "shipped")])).toBe(1);
  });

  it("names every stage", () => {
    expect([0, 1, 2, 3, 4].map(stageLabel)).toEqual(["Not started", "Plan", "Chats", "Pull request", "Shipped"]);
  });

  it("finds the trail a chat is on", () => {
    const trails = [trail("a", "inProgress", { sessionIds: ["s1"] }), trail("b", "inProgress", { sessionIds: ["s2"] })];
    expect(trailForSession(trails, "s2")?.id).toBe("b");
    expect(trailForSession(trails, "s3")).toBeUndefined();
    expect(trailForSession(trails, undefined)).toBeUndefined();
  });

  it("groups consecutive events into local days and names today and yesterday", () => {
    const now = new Date(2026, 10, 14, 12);
    const days = groupByDay([
      event("a", new Date(2026, 10, 9, 9).toISOString()),
      event("b", new Date(2026, 10, 9, 18).toISOString()),
      event("c", new Date(2026, 10, 13, 8).toISOString()),
      event("d", new Date(2026, 10, 14, 7).toISOString()),
    ], now);
    expect(days.map(day => day.events.map(item => item.id))).toEqual([["a", "b"], ["c"], ["d"]]);
    expect(days[1].label).toBe("Yesterday");
    expect(days[2].label).toBe("Today");
    expect(dayLabel("not a date", now)).toBe("Undated");
  });

  it("totals a brief selection and keeps the preview's order", () => {
    const items: TrailBriefItem[] = [
      { id: "note:n", label: "Note", tokens: 600, defaultOn: true },
      { id: "chat:s", label: "Chat", tokens: 2950, defaultOn: true },
      { id: "message:m", label: "Message", tokens: 64, defaultOn: false },
    ];
    const chosen = defaultBriefSelection(items);
    expect([...chosen]).toEqual(["note:n", "chat:s"]);
    expect(briefTotal(items, chosen)).toBe(3550);
    chosen.add("message:m");
    chosen.delete("note:n");
    expect(chosenInOrder(items, chosen)).toEqual(["chat:s", "message:m"]);
    expect([formatTokens(640), formatTokens(2940), formatTokens(3000), formatTokens(14400)]).toEqual(["640", "2.9k", "3k", "14k"]);
  });
});
