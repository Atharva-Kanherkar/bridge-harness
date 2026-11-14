// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { addPaneToChat, DOCK_ADDED_KEY, parseAddedPanes, pruneAddedPanes, readAddedPanes, resetAddedPanesCache, splitDockPanes } from "./dockVisibility";
import type { DockPaneId } from "./dockLayout";

const pane = (id: DockPaneId, extra: { available?: boolean; badge?: number; alert?: boolean } = {}) => ({ id, available: true, ...extra });
const strip = [pane("work"), pane("changes"), pane("code"), pane("terminal"), pane("browser"), pane("transcript"), pane("tasks"), pane("github")];
const ids = (panes: { id: DockPaneId }[]) => panes.map(item => item.id);

beforeEach(() => { localStorage.clear(); resetAddedPanesCache(); });
afterEach(() => { localStorage.clear(); resetAddedPanesCache(); });

describe("splitDockPanes", () => {
  it("shows the core panes and hides the rest, keeping the strip's order", () => {
    const { shown, hidden } = splitDockPanes(strip, { added: [] });
    expect(ids(shown)).toEqual(["work", "changes", "terminal", "browser"]);
    expect(ids(hidden)).toEqual(["code", "transcript", "tasks", "github"]);
  });

  it("hides a core pane that cannot work in this chat", () => {
    const noRepo = strip.map(item => item.id === "changes" || item.id === "terminal" ? { ...item, available: false } : item);
    expect(ids(splitDockPanes(noRepo, { added: [] }).shown)).toEqual(["work", "browser"]);
  });

  it("shows a pane with something to say", () => {
    const busy = strip.map(item => item.id === "tasks" ? { ...item, badge: 2 } : item.id === "github" ? { ...item, alert: true } : item);
    expect(ids(splitDockPanes(busy, { added: [] }).shown)).toEqual(["work", "changes", "terminal", "browser", "tasks", "github"]);
  });

  it("never hides the open pane, one opened before, or one added to this chat", () => {
    const { shown } = splitDockPanes(strip, { active: "transcript", added: ["code"], visited: ["github"] });
    expect(ids(shown)).toEqual(["work", "changes", "code", "terminal", "browser", "transcript", "github"]);
  });
});

describe("added panes", () => {
  it("are per chat and persist", () => {
    addPaneToChat("a", "code");
    addPaneToChat("a", "code");
    addPaneToChat("a", "github");
    expect(readAddedPanes("a")).toEqual(["code", "github"]);
    expect(readAddedPanes("b")).toEqual([]);
    expect(readAddedPanes(undefined)).toEqual([]);
    resetAddedPanesCache();
    expect(readAddedPanes("a")).toEqual(["code", "github"]);
    expect(JSON.parse(localStorage.getItem(DOCK_ADDED_KEY)!).a.panes).toEqual(["code", "github"]);
  });

  it("drops unknown panes and malformed entries on read", () => {
    expect(parseAddedPanes("nope")).toEqual({});
    expect(parseAddedPanes(JSON.stringify({ a: { panes: ["code", "made-up"], at: 1 }, b: { panes: "code", at: 1 }, c: { panes: ["code"] } }))).toEqual({ a: { panes: ["code"], at: 1 } });
  });

  it("remembers at most the most recently touched chats", () => {
    const stored = Object.fromEntries(Array.from({ length: 5 }, (_, index) => [`s${index}`, { panes: ["code" as DockPaneId], at: index }]));
    expect(Object.keys(pruneAddedPanes(stored, 3)).sort()).toEqual(["s2", "s3", "s4"]);
    for (let index = 0; index < 205; index += 1) addPaneToChat(`chat-${index}`, "code", index);
    const kept = Object.keys(JSON.parse(localStorage.getItem(DOCK_ADDED_KEY)!));
    expect(kept).toHaveLength(200);
    expect(kept).not.toContain("chat-0");
    expect(kept).toContain("chat-204");
  });
});
