import { beforeEach, describe, expect, it } from "vitest";
import type { Session, SessionStatus, Workspace } from "../types";
import {
  CHAT_VIEW_KEY,
  DEFAULT_CHAT_VIEW,
  agentOptions,
  chatTimestamp,
  dayLabel,
  filterChats,
  groupChats,
  readChatView,
  statusBucket,
  writeChatView,
} from "./sidebarChats";

const chat = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: null,
  harness: "codex",
  label: id,
  title: null,
  model: null,
  status: "idle" as SessionStatus,
  startedAt: "2026-08-19T09:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  kind: "chat",
  ...overrides,
} as Session);

const workspace = (id: string, title: string): Workspace => ({ id, title, status: "ready", dirtyFiles: 0 } as Workspace);

/** Local noon, so a day bucket never straddles a boundary because of the offset. */
const at = (year: number, month: number, day: number, hour = 12) =>
  new Date(year, month - 1, day, hour).toISOString();

const NOW = new Date(2026, 7, 19, 16, 0).getTime(); // 19 Aug 2026, local

describe("dayLabel", () => {
  it("names today and yesterday", () => {
    expect(dayLabel(NOW, NOW)).toBe("Today");
    expect(dayLabel(new Date(2026, 7, 18, 23, 30).getTime(), NOW)).toBe("Yesterday");
  });

  it("dates earlier days, adding the year only when it differs", () => {
    expect(dayLabel(new Date(2026, 7, 17, 9).getTime(), NOW)).toBe("Aug 17");
    expect(dayLabel(new Date(2025, 7, 17, 9).getTime(), NOW)).toBe("Aug 17, 2025");
  });

  it("still says yesterday across a spring-forward boundary", () => {
    // US spring forward 2026: 8 March. The 8th is 23 hours after the 7th, so a
    // fixed 86_400_000 subtraction would mislabel it.
    const now = new Date(2026, 2, 8, 12).getTime();
    expect(dayLabel(new Date(2026, 2, 7, 12).getTime(), now)).toBe("Yesterday");
  });
});

describe("chatTimestamp", () => {
  it("prefers startedAt, falls back to endedAt, and rejects nonsense", () => {
    expect(chatTimestamp(chat("a", { startedAt: at(2026, 8, 18) }))).toBe(Date.parse(at(2026, 8, 18)));
    expect(chatTimestamp(chat("b", { startedAt: null, endedAt: at(2026, 8, 17) }))).toBe(Date.parse(at(2026, 8, 17)));
    expect(chatTimestamp(chat("c", { startedAt: null, endedAt: null }))).toBeNull();
    expect(chatTimestamp(chat("d", { startedAt: "not a date" }))).toBeNull();
  });
});

describe("statusBucket", () => {
  it("maps lifecycle states onto the four filter buckets", () => {
    expect(statusBucket("working")).toBe("active");
    expect(statusBucket("warm")).toBe("active");
    expect(statusBucket("resuming")).toBe("active");
    expect(statusBucket("waiting")).toBe("waiting");
    expect(statusBucket("failed")).toBe("failed");
    expect(statusBucket("completed")).toBe("idle");
    expect(statusBucket("cancelled")).toBe("idle");
  });
});

describe("groupChats by date", () => {
  const chats = [
    chat("old", { startedAt: at(2026, 8, 17) }),
    chat("today-early", { startedAt: at(2026, 8, 19, 9) }),
    chat("yesterday", { startedAt: at(2026, 8, 18) }),
    chat("today-late", { startedAt: at(2026, 8, 19, 15) }),
  ];

  it("orders days newest first and chats newest first inside a day", () => {
    const groups = groupChats(chats, { groupBy: "date", sortBy: "recency", now: NOW });
    expect(groups.map(group => group.label)).toEqual(["Today", "Yesterday", "Aug 17"]);
    expect(groups[0].chats.map(item => item.id)).toEqual(["today-late", "today-early"]);
  });

  it("sinks an undated chat into a trailing Earlier group", () => {
    const groups = groupChats([...chats, chat("undated", { startedAt: null, endedAt: null })], {
      groupBy: "date",
      sortBy: "recency",
      now: NOW,
    });
    expect(groups.at(-1)).toMatchObject({ label: "Earlier" });
    expect(groups.at(-1)!.chats.map(item => item.id)).toEqual(["undated"]);
  });
});

describe("groupChats by project", () => {
  const workspaces = [workspace("ws-a", "harness"), workspace("ws-b", "agentclash"), workspace("ws-c", "unused")];
  const chats = [
    chat("a1", { workspaceId: "ws-a", startedAt: at(2026, 8, 19) }),
    chat("b1", { workspaceId: "ws-b", startedAt: at(2026, 8, 18) }),
    chat("loose", { workspaceId: null, startedAt: at(2026, 8, 19, 15) }),
  ];

  it("labels groups from workspace titles and trails the unassigned ones", () => {
    const groups = groupChats(chats, { groupBy: "project", sortBy: "recency", workspaces, now: NOW });
    expect(groups.map(group => group.label)).toEqual(["harness", "agentclash", "No project"]);
  });

  it("omits a workspace with no matching chat", () => {
    const groups = groupChats(chats, { groupBy: "project", sortBy: "recency", workspaces, now: NOW });
    expect(groups.map(group => group.label)).not.toContain("unused");
  });

  it("orders project groups alphabetically when sorting by name", () => {
    const groups = groupChats(chats, { groupBy: "project", sortBy: "name", workspaces, now: NOW });
    expect(groups.map(group => group.label)).toEqual(["agentclash", "harness", "No project"]);
  });
});

describe("groupChats by agent and status", () => {
  const chats = [
    chat("c1", { harness: "codex" }),
    chat("c2", { harness: "codex" }),
    chat("o1", { harness: "opencode" }),
    chat("k1", { harness: "claude" }),
  ];

  it("puts the busiest harness first", () => {
    const groups = groupChats(chats, { groupBy: "agent", sortBy: "recency", now: NOW });
    expect(groups.map(group => `${group.label}:${group.chats.length}`)).toEqual(["Codex:2", "Claude:1", "OpenCode:1"]);
  });

  it("keeps status buckets in a fixed order and drops empty ones", () => {
    const groups = groupChats(
      [
        chat("i", { status: "completed" }),
        chat("f", { status: "failed" }),
        chat("w", { status: "waiting" }),
        chat("a", { status: "warm" }),
      ],
      { groupBy: "status", sortBy: "recency", now: NOW },
    );
    expect(groups.map(group => group.label)).toEqual(["Active", "Waiting on you", "Failed", "Idle"]);
    expect(groups[0].chats[0].id).toBe("a");
  });
});

describe("groupChats ordering and none", () => {
  it("returns a single unlabelled group for none", () => {
    const groups = groupChats([chat("a"), chat("b")], { groupBy: "none", sortBy: "recency", now: NOW });
    expect(groups).toHaveLength(1);
    expect(groups[0].label).toBe("");
  });

  it("sorts by name on title, falling back to label", () => {
    const groups = groupChats(
      [chat("z", { title: "Zebra" }), chat("mid-label"), chat("a", { title: "Apple" })],
      { groupBy: "none", sortBy: "name", now: NOW },
    );
    expect(groups[0].chats.map(item => item.title ?? item.label)).toEqual(["Apple", "mid-label", "Zebra"]);
  });
});

describe("filterChats", () => {
  const chats = [
    chat("a", { title: "Policy engine budget", harness: "codex", model: "gpt-5.6-sol", status: "working" }),
    chat("b", { title: "Markdown tables", harness: "opencode", model: "qwen3.7-plus", status: "waiting" }),
    chat("c", { title: "Router docs", harness: "claude", model: "opus", status: "failed" }),
  ];

  it("matches title, label, harness label and model case-insensitively", () => {
    expect(filterChats(chats, { query: "POLICY" }).map(item => item.id)).toEqual(["a"]);
    expect(filterChats(chats, { query: "opencode" }).map(item => item.id)).toEqual(["b"]);
    expect(filterChats(chats, { query: "opus" }).map(item => item.id)).toEqual(["c"]);
    expect(filterChats(chats, { query: "b" }).map(item => item.id)).toEqual(["a", "b"]);
  });

  it("treats a blank query as no query", () => {
    expect(filterChats(chats, { query: "   " })).toHaveLength(3);
    expect(filterChats(chats)).toHaveLength(3);
  });

  it("filters by status bucket and by agent, and composes the two", () => {
    expect(filterChats(chats, { status: "active" }).map(item => item.id)).toEqual(["a"]);
    expect(filterChats(chats, { status: "failed" }).map(item => item.id)).toEqual(["c"]);
    expect(filterChats(chats, { agent: "opencode" }).map(item => item.id)).toEqual(["b"]);
    expect(filterChats(chats, { status: "active", agent: "opencode" })).toEqual([]);
  });
});

describe("agentOptions", () => {
  it("lists each harness once, alphabetically by label", () => {
    const options = agentOptions([chat("a", { harness: "opencode" }), chat("b", { harness: "codex" }), chat("c", { harness: "codex" })]);
    expect(options).toEqual([
      { id: "codex", label: "Codex" },
      { id: "opencode", label: "OpenCode" },
    ]);
  });
});

describe("chat view persistence", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, value); },
        removeItem: (key: string) => { store.delete(key); },
        clear: () => store.clear(),
      },
    });
  });

  it("defaults on empty storage and round-trips a written view", () => {
    expect(readChatView()).toEqual(DEFAULT_CHAT_VIEW);
    writeChatView({ status: "failed", agent: "claude", groupBy: "project", sortBy: "name" });
    expect(readChatView()).toEqual({ status: "failed", agent: "claude", groupBy: "project", sortBy: "name" });
  });

  it("falls back per field on an unknown value", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "bogus", agent: "codex", groupBy: "nope", sortBy: "name" }));
    expect(readChatView()).toEqual({ status: "all", agent: "codex", groupBy: "date", sortBy: "name" });
  });

  it("falls back wholesale on unparseable storage", () => {
    localStorage.setItem(CHAT_VIEW_KEY, "{not json");
    expect(readChatView()).toEqual(DEFAULT_CHAT_VIEW);
  });
});
