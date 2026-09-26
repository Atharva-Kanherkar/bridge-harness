import { beforeEach, describe, expect, it } from "vitest";
import type { Session, SessionStatus, Workspace } from "../types";
import { chatBucket, liveAgentSessions, BRIEFING_SESSION_KIND, EXTRACTION_SESSION_KIND, EVALUATION_SESSION_KIND, CONSOLIDATION_SESSION_KIND, CHAT_VIEW_KEY, DEFAULT_CHAT_VIEW, agentOptions, chatListTime, chatTimestamp, dayLabel, filterChats, groupChats, isHiddenSession, readChatView, statusBucket, visibleChats, writeChatView, SUGGESTION_SESSION_KIND } from "./sidebarChats";

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

describe("chatListTime", () => {
  const now = Date.parse("2026-08-19T16:00:00Z");

  it("buckets compact ages at the edges", () => {
    expect(chatListTime(now - 1_000, now)).toBe("now");
    expect(chatListTime(now - 4 * 60_000, now)).toBe("4m");
    expect(chatListTime(now - 2 * 3_600_000, now)).toBe("2h");
    expect(chatListTime(now - 3 * 86_400_000, now)).toBe("3d");
  });

  it("omits a missing or invalid timestamp so the row can stay quiet", () => {
    expect(chatListTime(null, now)).toBeNull();
    expect(chatListTime(Number.NaN, now)).toBeNull();
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

  it("matches a chat on its project name when a resolver is supplied", () => {
    const inProject = chat("p", { title: "Token cost report", workspaceId: "ws-1" });
    const resolver = (id: string | null | undefined) => (id === "ws-1" ? "agentclash" : undefined);
    expect(filterChats([...chats, inProject], { query: "agentclash", workspaceTitle: resolver }).map(item => item.id)).toEqual(["p"]);
    // Without the resolver the project name is simply not searchable.
    expect(filterChats([...chats, inProject], { query: "agentclash" })).toEqual([]);
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
    expect(readChatView()).toEqual({ status: "all", agent: "codex", groupBy: "project", sortBy: "name" });
  });

  it("falls back wholesale on unparseable storage", () => {
    localStorage.setItem(CHAT_VIEW_KEY, "{not json");
    expect(readChatView()).toEqual(DEFAULT_CHAT_VIEW);
  });

  it("defaults groupBy to project", () => {
    expect(DEFAULT_CHAT_VIEW.groupBy).toBe("project");
  });
});

describe("hidden sessions", () => {
  const chat = (id: string, kind: string | null = null): Session =>
    ({ id, workspaceId: null, harness: "codex", label: id, title: id, status: "idle", kind } as unknown as Session);

  it("hides every run Bridge does for itself, and nothing else", () => {
    expect(isHiddenSession(chat("a", BRIEFING_SESSION_KIND))).toBe(true);
    expect(isHiddenSession(chat("s", SUGGESTION_SESSION_KIND))).toBe(true);
    expect(isHiddenSession(chat("x", EXTRACTION_SESSION_KIND))).toBe(true);
    expect(isHiddenSession(chat("e", EVALUATION_SESSION_KIND))).toBe(true);
    expect(isHiddenSession(chat("c", CONSOLIDATION_SESSION_KIND))).toBe(true);
    for (const kind of [null, "orchestrator", "chat", "worker", "direct"]) {
      expect(isHiddenSession(chat("b", kind))).toBe(false);
    }
  });

  it("filters hidden sessions out of a list without disturbing the order of the rest", () => {
    const chats = [
      chat("first"),
      chat("briefing-1", BRIEFING_SESSION_KIND),
      chat("second", "orchestrator"),
      chat("suggestion-1", SUGGESTION_SESSION_KIND),
      chat("third"),
    ];
    expect(visibleChats(chats).map(item => item.id)).toEqual(["first", "second", "third"]);
  });

  it("keeps a briefing session out of every surface that filters through here", () => {
    // The contract named `a_briefing_session_is_hidden_from_every_surface`, and this is
    // it: the rail, Agent Fleet and default selection all read the list this
    // predicate produces, so one assertion covers all three. Review caught that the
    // name existed in the contract and nowhere else.
    const briefing = chat("briefing-1", BRIEFING_SESSION_KIND);
    const chats = [chat("plain"), briefing, chat("orchestrated", "orchestrator")];
    const visible = visibleChats(chats);
    expect(visible).not.toContain(briefing);
    expect(visible.map(item => item.id)).toEqual(["plain", "orchestrated"]);
  });

  it("names the same kinds the backend does", () => {
    // The Rust side owns these literals; if they ever diverge, a hidden session
    // becomes visible in the rail, which is the one place it must never appear.
    expect(BRIEFING_SESSION_KIND).toBe("briefing");
    expect(SUGGESTION_SESSION_KIND).toBe("suggestion");
    expect(EVALUATION_SESSION_KIND).toBe("outcome_evaluation");
    expect(CONSOLIDATION_SESSION_KIND).toBe("consolidation");
  });
});

describe("live agents under an idle chat", () => {
  const root = chat("root", { status: "ready" as SessionStatus });
  const worker = (id: string, parentSessionId: string, status: SessionStatus) => chat(id, { kind: "worker", parentSessionId, status });

  it("counts live descendants toward the chat at the top of the tree", () => {
    const live = liveAgentSessions([
      root,
      worker("w1", "root", "working" as SessionStatus),
      worker("w2", "root", "waiting" as SessionStatus),
      worker("w3", "w1", "working" as SessionStatus),
      worker("w4", "root", "ready" as SessionStatus),
      worker("w5", "root", "stopped" as SessionStatus),
    ]);
    expect(live.get("root")).toEqual(["w1", "w2", "w3"]);
    expect(live.has("w1")).toBe(false);
  });

  it("moves an idle chat into Active while its agents run, and nowhere else", () => {
    const live = liveAgentSessions([root, worker("w1", "root", "working" as SessionStatus)]);
    expect(chatBucket(root, live)).toBe("active");
    expect(chatBucket(root, new Map())).toBe("idle");
    expect(chatBucket(chat("asking", { status: "waiting" as SessionStatus }), new Map([["asking", ["w9"]]]))).toBe("waiting");
    expect(filterChats([root, chat("quiet")], { status: "active", liveAgents: live }).map(item => item.id)).toEqual(["root"]);
    expect(groupChats([root], { groupBy: "status", sortBy: "recency", now: NOW, liveAgents: live }).map(group => group.key)).toEqual(["active"]);
  });

  it("clears as soon as the last agent settles", () => {
    expect(liveAgentSessions([root, worker("w1", "root", "completed" as SessionStatus)]).size).toBe(0);
  });
});
