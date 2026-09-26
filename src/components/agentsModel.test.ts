import { describe, expect, it } from "vitest";
import type { AgentEvent, Session, SessionForestSnapshot, WorkerRuntimeRecord } from "../types";
import type { ConversationItem } from "../transcript/item";
import { agentsModel, agentTrail, censusLine, findAgent } from "./agentsModel";

// Contract: testing/feat-agents-pane.md §1.

const NOW = Date.parse("2026-08-25T10:05:00Z");

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, harness: "claude", label: id, status: "working", kind: "orchestrator", metricSource: "reported",
  restorationMode: "hot", continuationFidelity: "native", startedAt: "2026-08-25T10:00:00Z", ...overrides,
} as Session);

const worker = (id: string, overrides: Partial<Session> = {}): Session =>
  session(id, { kind: "worker", parentSessionId: "root", label: `Worker ${id}`, ...overrides } as Partial<Session>);

const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId, parentSessionId: "root", lifecycleState: "working", taskFamily: "implementation",
  compatibilityKey: "k", resultStatus: "pending", retryCount: 0, lastResult: null,
  lastActivityAt: "2026-08-25T10:00:00Z", updatedAt: "2026-08-25T10:00:00Z", ...overrides,
} as WorkerRuntimeRecord);

const forest = (overrides: Partial<SessionForestSnapshot> = {}): SessionForestSnapshot => ({
  sessionId: "root", entries: [], leaves: [], workerLeases: [], workerRuntimes: [], workerQueue: [],
  entryWindow: { returned: 0, total: 0, trimmedPayloads: 0 }, policyLimits: {} as never,
  reasons: [], repositoryDivergence: {} as never, usage: [], ...overrides,
} as unknown as SessionForestSnapshot);

const event = (overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id: 1, sessionId: "root", kind: "tool.completed" as unknown as AgentEvent["kind"], protocolVersion: 1,
  sequence: 1, createdAt: "2026-08-25T10:00:30Z", role: "assistant", status: "completed",
  text: null, title: null, data: {}, providerMeta: {}, ...overrides,
} as AgentEvent);

const item = (overrides: Partial<ConversationItem> = {}): ConversationItem => ({
  key: "k1", type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", data: {}, ...overrides,
} as ConversationItem);

describe("agentsModel", () => {
  it("merges Bridge workers and harness subagents into one tree", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1"), worker("w2"), worker("w3")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1"), runtime("w2"), runtime("w3")] })]]),
      transcripts: new Map([["root", [
        item({
          key: "sub", sequence: 2,
          data: { subagent: { sessionId: "toolu_1", agent: "Explore", title: "Map the store" } },
        }),
      ]]]),
      rootSessionId: "root",
    });
    expect(runs).toHaveLength(1);
    const [run] = runs;
    expect(run.rootSessionId).toBe("root");
    expect(run.agents.filter(node => node.source === "worker").map(node => node.id)).toEqual(["w1", "w2", "w3"]);
    const subagent = run.agents.find(node => node.source === "subagent");
    expect(subagent?.name).toBe("Explore");
    expect(run.census.workers).toBe(3);
    expect(run.census.subagents).toBe(1);
  });

  it("nests a subagent under the worker whose transcript it arrived in", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1")] })]]),
      transcripts: new Map([["w1", [item({ key: "s", data: { subagent: { sessionId: "toolu_9", agent: "Explore" } } })]]]),
      events: [event({ id: 2, sessionId: "w1", kind: "tool.completed" as unknown as AgentEvent["kind"], status: "completed", title: "Read tokenStore.ts", data: { type: "readFile", path: "src/auth/tokenStore.ts" } })],
      rootSessionId: "root",
    });
    const child = runs[0].agents[0].children[0];
    expect(child.parentId).toBe("w1");
    expect(child.depth).toBe(1);
    expect(child.source).toBe("subagent");
  });

  it("leaves an orchestrator's own subagent at depth 0", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root")],
      forests: new Map([["root", forest()]]),
      transcripts: new Map([["root", [item({ data: { subagent: { sessionId: "toolu_1", agent: "general-purpose" } } })]]]),
      rootSessionId: "root",
    });
    expect(runs[0].agents[0].depth).toBe(0);
    expect(runs[0].agents[0].parentId).toBeUndefined();
  });

  it("counts the census across the whole run", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1"), worker("w2")],
      forests: new Map([["root", forest({
        workerRuntimes: [
          runtime("w1"),
          runtime("w2", { lifecycleState: "completed", resultStatus: "reported", lastResult: { status: "completed", summary: "done" } }),
        ],
        workerQueue: [{ id: "q", actualModel: "m" } as never],
      })]]),
      rootSessionId: "root",
    });
    expect(runs[0].census).toMatchObject({ running: 1, done: 1, queued: 1, failed: 0, needsYou: 0 });
    expect(censusLine(runs[0].census)).toBe("1 running · 1 done");
    expect(censusLine({ running: 0, needsYou: 0, done: 0, failed: 0, queued: 0, workers: 0, subagents: 0, costUsd: 0 })).toBe("");
  });

  it("reads a missing runtime as a quiet start, never as attention", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [{ ...runtime("w1"), lifecycleState: "starting" } as WorkerRuntimeRecord] })]]),
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.status.label).toBe("STARTING");
    expect(node.status.tone).not.toBe("attention");
  });

  it("never lets a machine fence become a live line", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1")] })]]),
      events: [event({ id: 3, sessionId: "w1", text: "```bridge-worker-result" })],
      rootSessionId: "root",
    });
    expect(runs[0].agents[0].liveLine).toBeUndefined();
  });

  it("keeps a finished row, with the time it took", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1", { endedAt: "2026-08-25T10:01:00Z" })],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1", { resultStatus: "reported", lastResult: { status: "completed", summary: "shipped" } })] })]]),
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.status.tone).toBe("done");
    expect(node.endedAt).toBe("2026-08-25T10:01:00Z");
    expect(runs[0].census.done).toBe(1);
  });

  it("dims an acknowledged failure rather than hiding it", () => {
    const input = {
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1", { lifecycleState: "failed", failureClass: "sandbox_denied" })] })]]),
      rootSessionId: "root",
    };
    expect(agentsModel(input)[0].agents[0].acknowledged).toBe(false);
    const acknowledged = agentsModel({ ...input, acknowledged: new Set(["w1"]) })[0].agents[0];
    expect(acknowledged.acknowledged).toBe(true);
    expect(acknowledged.failureCode).toBe("sandbox_denied");
  });

  it("filters by scope", () => {
    const sessions = [session("root"), session("other", { title: "Other" }), worker("w1"), worker("w2", { parentSessionId: "other" })];
    const forests = new Map([
      ["root", forest({ workerRuntimes: [runtime("w1")] })],
      ["other", forest({ sessionId: "other", workerRuntimes: [runtime("w2", { parentSessionId: "other" })] })],
    ]);
    expect(agentsModel({ now: NOW, sessions, forests, rootSessionId: "root" }).map(run => run.rootSessionId)).toEqual(["root"]);
    expect(agentsModel({ now: NOW, sessions, forests, rootSessionId: "root", scope: "all-chats" }).map(run => run.rootSessionId).sort()).toEqual(["other", "root"]);
  });

  it("takes a subagent's live line from its own rows, never the parent's", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root")],
      forests: new Map([["root", forest()]]),
      transcripts: new Map([["root", [
        item({ key: "a", sequence: 1, data: { subagent: { sessionId: "toolu_1", agent: "Explore" } }, tool: { verb: "read", doing: "Reading", done: "Read", glyph: "file", status: "completed", path: "src/auth/client.ts", target: "client.ts" } as never }),
        item({ key: "b", sequence: 2, data: {}, tool: { verb: "read", doing: "Reading", done: "Read", glyph: "file", status: "completed", target: "unrelated.ts" } as never }),
      ]]]),
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.steps).toHaveLength(1);
    expect(node.liveLine?.target).toBe("client.ts");
  });

  it("gives a Codex collab row a status, with only the Task call as a step", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root", { harness: "codex" })],
      forests: new Map([["root", forest()]]),
      transcripts: new Map([["root", [item({
        key: "collab", type: "activity", status: "inProgress",
        tool: { verb: "tool", doing: "Delegating", done: "Delegated", glyph: "fork", status: "running", subagent: { agentType: "explorer", status: "running" } } as never,
      })]]]),
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.name).toBe("explorer");
    expect(node.status.tone).toBe("working");
    // The child has no attributed tool rows of its own — the app-server reports
    // its lifecycle, not its work — so the parent's Task call is the one real
    // step, and nothing is invented to fill the gap.
    expect(node.steps).toEqual([{ id: "collab", verb: "Task", target: "explorer", extra: "running", state: "live" }]);
  });

  it("reads the live line from the worker's own tool frames", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1")] })]]),
      events: [
        event({ id: 4, sessionId: "w1", kind: "file_change.completed" as unknown as AgentEvent["kind"], title: "store.rs", data: { path: "src/auth/store.rs", additions: 18, deletions: 6 } }),
        event({ id: 5, sessionId: "w1", kind: "tool.completed" as unknown as AgentEvent["kind"], title: "Read tokenStore.ts", data: { type: "readFile", path: "src/auth/tokenStore.ts" } }),
      ],
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.steps.map(step => step.verb)).toEqual(["Edit", "Read"]);
    expect(node.liveLine).toMatchObject({ verb: "Read", target: "tokenStore.ts" });
    expect(node.steps[0]).toMatchObject({ extra: "+18 −6", additions: 18, deletions: 6 });
    expect(node.counters.additions).toBe(18);
    expect(node.counters.deletions).toBe(6);
  });

  it("reads a child's ask off the orchestrator's transcript", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1")] })]]),
      transcripts: new Map([["root", [item({
        key: "ask", type: "delegation", eventId: 42, status: "pending",
        title: "Implementation needs your approval", text: "Run the suite?",
        data: { childBlocked: true, childSessionId: "w1", command: "bun test src/auth", cwd: ".worktrees/w1", ownedPaths: ["src/auth/**"] },
      })]]]),
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.ask).toMatchObject({ eventId: 42, command: "bun test src/auth", cwd: ".worktrees/w1", ownedPaths: ["src/auth/**"] });
    expect(runs[0].census.needsYou).toBe(1);
  });

  it("shows a stable failure code and never the provider's own text", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1", { lifecycleState: "failed", failureClass: "sandbox_denied" })] })]]),
      events: [event({ id: 6, sessionId: "w1", kind: "file_change.completed" as unknown as AgentEvent["kind"], status: "failed", title: ".github/workflows/ci.yml", data: { path: ".github/workflows/ci.yml" } })],
      rootSessionId: "root",
    });
    const node = runs[0].agents[0];
    expect(node.failureCode).toBe("sandbox_denied");
    expect(node.liveLine?.target).toBe("ci.yml");
    expect(JSON.stringify(node)).not.toContain("ENOENT");
  });

  it("finds a node and its trail from a chat pointer", () => {
    const runs = agentsModel({
      now: NOW,
      sessions: [session("root"), worker("w1")],
      forests: new Map([["root", forest({ workerRuntimes: [runtime("w1")] })]]),
      transcripts: new Map([["w1", [item({ data: { subagent: { sessionId: "toolu_1", agent: "Explore" } } })]]]),
      rootSessionId: "root",
    });
    const childId = runs[0].agents[0].children[0].id;
    expect(findAgent(runs, childId)?.name).toBe("Explore");
    expect(agentTrail(runs, childId).map(node => node.name)).toEqual(["Worker w1", "Explore"]);
    expect(findAgent(runs, "nope")).toBeUndefined();
  });
});
