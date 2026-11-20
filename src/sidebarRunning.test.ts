import { describe, expect, it } from "vitest";
import { asWireKind } from "./transcript/wire";
import type { AgentEvent, Session } from "./types";
import { actionLabel, formatElapsed, liveAction, rememberRunning, runningChats, runningDetail, turnInFlight, turnStartedAt, type RunningChat } from "./sidebarRunning";

const NOW = Date.parse("2026-11-21T10:00:00Z");

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: null,
  harness: "codex",
  label: id,
  title: id,
  status: "ready",
  startedAt: "2026-11-21T09:00:00Z",
  endedAt: null,
  activeTurnId: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  kind: "orchestrator",
  ...overrides,
} as Session);

let nextId = 1;
const frame = (sessionId: string, kind: string, fields: Partial<AgentEvent> = {}): AgentEvent => {
  const id = nextId++;
  return {
    id, sessionId, sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null,
    title: null, text: null, data: {}, providerMeta: {}, createdAt: new Date(NOW - 60_000).toISOString(), ...fields,
  };
};

const ids = (rows: RunningChat[]) => rows.map(row => row.chat.id);

describe("turnInFlight", () => {
  it("is a provider-acknowledged turn or a send on its way, never the status alone", () => {
    expect(turnInFlight({ activeTurnId: "turn-1" })).toBe(true);
    expect(turnInFlight({ activeTurnId: null }, true)).toBe(true);
    expect(turnInFlight({ activeTurnId: null })).toBe(false);
    // A provider that is up with nothing to do writes `working` too.
    expect(turnInFlight(session("up", { status: "working", activeTurnId: null }))).toBe(false);
  });
});

describe("runningChats membership", () => {
  it("lists a chat with a turn in flight and leaves out one whose provider is merely up", () => {
    const rows = runningChats({
      chats: [
        session("turn", { status: "working", activeTurnId: "turn-1" }),
        session("provider-up", { status: "working", activeTurnId: null }),
        session("idle", { status: "ready" }),
      ],
      now: NOW,
    });
    expect(ids(rows)).toEqual(["turn"]);
    expect(rows[0].turn).toBe(true);
  });

  it("lists a chat the moment a send to it is on its way", () => {
    const rows = runningChats({ chats: [session("sending")], sending: new Set(["sending"]), now: NOW });
    expect(ids(rows)).toEqual(["sending"]);
    expect(rows[0].turn).toBe(true);
  });

  it("lists a chat whose own turn ended while its workers still run, with their states", () => {
    const chat = session("orchestrator");
    const rows = runningChats({
      chats: [chat],
      liveAgents: new Map([["orchestrator", ["w1", "w2", "w3"]]]),
      sessions: [
        chat,
        session("w1", { parentSessionId: "orchestrator", status: "working", kind: "worker" }),
        session("w2", { parentSessionId: "orchestrator", status: "waiting", kind: "worker" }),
        session("w3", { parentSessionId: "w1", status: "working", kind: "worker" }),
      ],
      now: NOW,
    });
    expect(ids(rows)).toEqual(["orchestrator"]);
    expect(rows[0]).toMatchObject({ turn: false, workers: 3, workersWaiting: 1, action: null });
  });

  it("counts live workers even without their sessions, as working", () => {
    const rows = runningChats({ chats: [session("orchestrator")], liveAgents: new Map([["orchestrator", ["w1"]]]), now: NOW });
    expect(rows[0]).toMatchObject({ workers: 1, workersWaiting: 0 });
  });

  it("is empty when nothing runs", () => {
    expect(runningChats({ chats: [], now: NOW })).toEqual([]);
    expect(runningChats({ chats: [session("a"), session("b", { status: "working" })], liveAgents: new Map([["a", []]]), now: NOW })).toEqual([]);
  });
});

describe("runningChats clocks and order", () => {
  it("times a turn from its turn.started frame, else from when this view first saw it", () => {
    const started = NOW - 13_000;
    const events = [
      frame("timed", "turn.started", { createdAt: new Date(started).toISOString() }),
      frame("other", "turn.started", { createdAt: new Date(NOW - 99_000).toISOString() }),
    ];
    const rows = runningChats({
      chats: [session("timed", { activeTurnId: "t" }), session("seen", { activeTurnId: "t" }), session("fresh", { activeTurnId: "t" })],
      events,
      seen: new Map([["seen", NOW - 72_000]]),
      now: NOW,
    });
    const since = Object.fromEntries(rows.map(row => [row.chat.id, row.since]));
    expect(since).toEqual({ timed: started, seen: NOW - 72_000, fresh: NOW });
  });

  it("does not time a turn from a start that an end came after", () => {
    const events = [frame("a", "turn.started"), frame("a", "turn.completed", { status: "completed" })];
    expect(turnStartedAt(events)).toBeNull();
    const rows = runningChats({ chats: [session("a", { activeTurnId: "t2" })], events, seen: new Map([["a", NOW - 5_000]]), now: NOW });
    expect(rows[0].since).toBe(NOW - 5_000);
  });

  it("times a workers-only run from the earliest live worker when it was not seen start", () => {
    const chat = session("orchestrator");
    const rows = runningChats({
      chats: [chat],
      liveAgents: new Map([["orchestrator", ["w1", "w2"]]]),
      sessions: [
        session("w1", { status: "working", startedAt: new Date(NOW - 30_000).toISOString() }),
        session("w2", { status: "working", startedAt: new Date(NOW - 72_000).toISOString() }),
      ],
      now: NOW,
    });
    expect(rows[0].since).toBe(NOW - 72_000);
    // Seen running already (its turn just ended): the clock carries on.
    const carried = runningChats({ chats: [chat], liveAgents: new Map([["orchestrator", ["w1"]]]), seen: new Map([["orchestrator", NOW - 5_000]]), now: NOW });
    expect(carried[0].since).toBe(NOW - 5_000);
  });

  it("puts the newest run first", () => {
    const rows = runningChats({
      chats: [session("old", { activeTurnId: "t" }), session("new", { activeTurnId: "t" }), session("middle", { activeTurnId: "t" })],
      seen: new Map([["old", NOW - 90_000], ["new", NOW - 1_000], ["middle", NOW - 30_000]]),
      now: NOW,
    });
    expect(ids(rows)).toEqual(["new", "middle", "old"]);
  });

  it("carries one clock per running chat and drops the ones that stopped", () => {
    const rows = runningChats({ chats: [session("a", { activeTurnId: "t" })], seen: new Map([["a", NOW - 4_000], ["stopped", NOW - 9_000]]), now: NOW });
    expect([...rememberRunning(rows)]).toEqual([["a", NOW - 4_000]]);
  });
});

describe("runningChats current action", () => {
  it("names the call in flight the way the live card does", () => {
    const events = [
      frame("a", "turn.started"),
      frame("a", "tool.completed", { itemId: "read", status: "completed", title: "Read store.rs", data: { type: "readFile", path: "src/store.rs" } }),
      frame("a", "tool.started", { itemId: "run", status: "inProgress", data: { type: "commandExecution", command: "cargo test" } }),
    ];
    const [row] = runningChats({ chats: [session("a", { activeTurnId: "t" })], events, now: NOW });
    expect(row.action).toBe("Running cargo test");
    expect(runningDetail(row)).toEqual({ text: "Running cargo test", tone: "live" });
  });

  it("drops a dangling flag from a shortened command, and keeps the harness's own account", () => {
    const run = (command: string, extra: Record<string, unknown> = {}) => liveAction([frame("a", "tool.started", { itemId: "run", status: "inProgress", data: { type: "commandExecution", command, ...extra } })]);
    expect(run("cargo test -p bridge-core policy::")).toBe("Running cargo test");
    expect(run("npm install")).toBe("Running npm install");
    expect(run("rg -n retry src")).toBe("Searching “retry”");
    expect(actionLabel({ verb: "run", glyph: "terminal", doing: "Running", done: "Ran", target: "cargo test -p", command: "cargo test -p x", intent: "Run the policy tests", status: "running" })).toBe("Run the policy tests");
  });

  it("has no action when nothing is running, and ignores other chats' calls", () => {
    expect(liveAction([])).toBeNull();
    const events = [frame("other", "tool.started", { itemId: "run", status: "inProgress", data: { type: "commandExecution", command: "cargo test" } })];
    const [row] = runningChats({ chats: [session("a", { activeTurnId: "t" })], events, now: NOW });
    expect(row.action).toBeNull();
  });

  it("reads no action for a workers-only run: its own calls are over", () => {
    const events = [frame("a", "tool.started", { itemId: "run", status: "inProgress", data: { type: "commandExecution", command: "cargo test" } })];
    const [row] = runningChats({ chats: [session("a")], liveAgents: new Map([["a", ["w"]]]), events, now: NOW });
    expect(row.action).toBeNull();
  });
});

describe("runningDetail", () => {
  const row = (overrides: Partial<RunningChat>): RunningChat => ({ chat: session("a"), since: NOW, turn: true, workers: 0, workersWaiting: 0, needsYou: false, action: null, ...overrides });

  it("puts the person first, then the step, then the workers", () => {
    expect(runningDetail(row({ needsYou: true, action: "Running x" }))).toEqual({ text: "Needs you", tone: "needsYou" });
    expect(runningDetail(row({ workers: 3, workersWaiting: 1, action: "Running x" }))).toEqual({ text: "1 worker needs you", tone: "needsYou" });
    expect(runningDetail(row({ workers: 3, workersWaiting: 2 })).text).toBe("2 workers need you");
    expect(runningDetail(row({ workers: 2, action: "Editing a.ts" })).text).toBe("Editing a.ts");
    expect(runningDetail(row({ workers: 1 })).text).toBe("1 worker working");
    expect(runningDetail(row({ workers: 3, turn: false })).text).toBe("3 workers working");
    expect(runningDetail(row({}))).toEqual({ text: "Working", tone: "live" });
  });
});

describe("formatElapsed", () => {
  it("reads as a stopwatch", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(13_400)).toBe("0:13");
    expect(formatElapsed(72_000)).toBe("1:12");
    expect(formatElapsed(3_723_000)).toBe("1:02:03");
    expect(formatElapsed(-5_000)).toBe("0:00");
    expect(formatElapsed(Number.NaN)).toBe("0:00");
  });
});
