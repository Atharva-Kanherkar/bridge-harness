import { describe, expect, it } from "vitest";
import { chatConversationItems, chatOverview, checksRow, divergencePhrase, formatTurnDuration, overviewPath, turnChanges, turnHistory, turnTitle } from "./chatOverview";
import { asWireKind } from "./transcript/wire";
import type { AgentEvent, BaseBranchDivergence, CompletionSummary } from "./types";

const at = (seconds: number) => new Date(Date.parse("2026-11-20T10:00:00Z") + seconds * 1000).toISOString();

let nextId = 0;
const event = (kind: string, fields: Partial<AgentEvent> = {}): AgentEvent => {
  nextId += 1;
  return {
    id: nextId, sessionId: "s", sequence: nextId, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${nextId}`,
    role: null, status: "completed", title: null, text: null, data: {}, providerMeta: { adapter: "codex" },
    createdAt: at(nextId), ...fields,
  };
};
const prompt = (text: string, seconds: number) => event("message.completed", { role: "user", text, createdAt: at(seconds) });
const reply = (text: string, seconds: number) => event("message.completed", { role: "assistant", text, createdAt: at(seconds) });
const edit = (path: string, additions: number | undefined, deletions: number | undefined, seconds: number, extra: Partial<AgentEvent> = {}) =>
  event("file_change.completed", { title: path, createdAt: at(seconds), data: { path, additions, deletions, durationMs: 1000 }, ...extra });
const read = (path: string, seconds: number, durationMs = 2000) =>
  event("tool.completed", { title: `Read ${path}`, createdAt: at(seconds), data: { type: "readFile", path, durationMs } });
const command = (text: string, exitCode: number, output: string, seconds: number) =>
  event("command.completed", { title: text, createdAt: at(seconds), data: { type: "commandExecution", command: text, exitCode, durationMs: 4000, aggregatedOutput: output } });

const twoTurns = () => [
  prompt("Read the policy engine\nand tell me where retries live.", 0),
  read("src/policy.rs", 2, 3000),
  read("src/policy_coordinator.rs", 6, 2000),
  reply("Retries live in the coordinator.", 9),
  prompt("  Add a retry budget   to the policy engine. Run the tests.", 20),
  edit("src/policy.rs", 10, 2, 22),
  edit("src/policy.rs", 5, 1, 25),
  edit("src/PolicyCard.tsx", 3, 0, 27),
  edit("src/broken.rs", 40, 40, 28, { status: "failed" }),
  command("cargo test -p bridge-core policy::", 0, "test result: ok. 42 passed; 0 failed; 0 ignored", 30),
  reply("Added the budget.", 40),
];

const divergence = (over: Partial<BaseBranchDivergence>): BaseBranchDivergence => ({
  ahead: 0, behind: 0, baseRef: "origin/main", branch: "bridge/retry-budget", dirty: false, fetchAttempted: false, fetched: false, unavailableReason: null, ...over,
});

const completion = (over: Partial<CompletionSummary>): CompletionSummary => ({
  attemptId: "a", contractId: "c", verdict: "verified", markdownCommitted: false, waiverReason: null,
  repository: { head: "abc", dirtyDigest: "clean" }, passedRequired: 2, totalRequired: 2,
  checks: [
    { checkId: "rust", kind: "deterministic", required: true, status: "passed", executor: "bridge.shell", command: "cargo test", verifierFamily: null, detail: null, outputDigest: null, artifactRefs: [] },
    { checkId: "build", kind: "deterministic", required: true, status: "passed", executor: "bridge.shell", command: "bun run build", verifierFamily: null, detail: null, outputDigest: null, artifactRefs: [] },
  ],
  ...over,
});

describe("chat overview rows", () => {
  it("hides every row whose fact is absent", () => {
    const model = chatOverview({ items: [], turnActive: false });
    expect(model.rows).toEqual([]);
    expect(model.turns).toEqual([]);
    expect(model.headline).toBeUndefined();
  });

  it("reads where, branch, changes and checks when each is known", () => {
    const model = chatOverview({
      path: "/Users/you/Developer/bridge/.worktrees/retry-budget",
      projectName: "bridge",
      branch: "bridge/retry-budget",
      divergence: divergence({ ahead: 2 }),
      changes: { files: 4, additions: 86, deletions: 12 },
      completion: completion({}),
      items: [],
      turnActive: false,
    });
    expect(model.rows.map(row => row.kind)).toEqual(["where", "branch", "changes", "checks"]);
    expect(model.rows[0]).toEqual({ kind: "where", path: "/Users/you/Developer/bridge/.worktrees/retry-budget", display: "…/bridge/.worktrees/retry-budget", detail: "bridge" });
    expect(model.rows[1]).toEqual({ kind: "branch", name: "bridge/retry-budget", detail: "2 ahead of origin/main" });
    expect(model.rows[2]).toEqual({ kind: "changes", files: 4, additions: 86, deletions: 12 });
    expect(model.rows[3]).toMatchObject({ kind: "checks", tone: "passed", label: "Verified", detail: "2 of 2 required checks passed" });
    expect(model.headline).toBe("Changes to review");
  });

  it("keeps a clean tree as a fact, and has no changes row without a repository", () => {
    expect(chatOverview({ changes: { files: 0, additions: 0, deletions: 0 }, items: [], turnActive: false }).rows).toEqual([{ kind: "changes", files: 0, additions: 0, deletions: 0 }]);
    expect(chatOverview({ changes: { files: 0, additions: 0, deletions: 0 }, items: [], turnActive: false }).headline).toBe("No uncommitted changes");
    expect(chatOverview({ changes: null, items: [], turnActive: false }).rows).toEqual([]);
  });

  it("says Working while a turn is in flight, ahead of every other headline", () => {
    expect(chatOverview({ changes: { files: 2, additions: 1, deletions: 1 }, completion: completion({ verdict: "failed" }), items: [], turnActive: true }).headline).toBe("Working");
    expect(chatOverview({ changes: { files: 2, additions: 1, deletions: 1 }, completion: completion({ verdict: "failed" }), items: [], turnActive: false }).headline).toBe("Checks failed");
  });

  it("prefers the branch the base comparison measured, and drops ahead/behind it could not measure", () => {
    expect(chatOverview({ branch: "stored", divergence: divergence({ branch: "measured" }), items: [], turnActive: false }).rows[0]).toMatchObject({ name: "measured", detail: "Up to date with origin/main" });
    expect(chatOverview({ branch: "stored", divergence: divergence({ branch: "measured", unavailableReason: "no base" }), items: [], turnActive: false }).rows[0]).toEqual({ kind: "branch", name: "stored", detail: undefined });
  });

  it("words ahead and behind the way they read", () => {
    expect(divergencePhrase(divergence({ ahead: 1, behind: 67 }))).toBe("1 ahead, 67 behind origin/main");
    expect(divergencePhrase(divergence({ behind: 3 }))).toBe("3 behind origin/main");
    expect(divergencePhrase(divergence({ baseRef: null }))).toBeUndefined();
    expect(divergencePhrase(null)).toBeUndefined();
  });

  it("shortens a long path to its leaf directories and keeps a short one whole", () => {
    expect(overviewPath("/a/b/c/d/e")).toBe("…/c/d/e");
    expect(overviewPath("/tmp/repo")).toBe("/tmp/repo");
  });
});

describe("checks row", () => {
  it("names the failing required check over the counts", () => {
    const failed = completion({
      verdict: "failed", passedRequired: 1,
      checks: [...completion({}).checks.slice(0, 1), { ...completion({}).checks[1], status: "failed" }],
    });
    expect(checksRow(failed, [])).toEqual({ kind: "checks", tone: "failed", label: "Verification failed", detail: "bun run build failed · 1 of 2 required checks passed" });
  });

  it("reads an in-flight proof as pending, claiming nothing", () => {
    expect(checksRow(completion({ verdict: "verifying", passedRequired: 1 }), [])).toMatchObject({ tone: "pending", label: "Verifying" });
  });

  it("falls back to the newest settled check run in the chat", () => {
    const items = chatConversationItems(undefined, null, [
      prompt("go", 0),
      command("bun run build", 1, "error TS2322", 1),
      command("cargo test -p bridge-core policy::", 0, "test result: ok. 42 passed; 0 failed; 0 ignored", 2),
      command("ls", 0, "a b", 3),
    ]);
    expect(checksRow(null, items)).toMatchObject({ kind: "checks", tone: "passed", label: "cargo test -p bridge-core policy::" });
    expect(checksRow(null, items)?.detail).toMatch(/^Passed/);
    expect(checksRow(null, chatConversationItems(undefined, null, [prompt("go", 0), command("bun run build", 1, "error TS2322", 1)]))).toMatchObject({ tone: "failed", label: "bun run build", detail: "Failed" });
  });

  it("shows no checks row when nothing was verified", () => {
    expect(checksRow(null, chatConversationItems(undefined, null, [prompt("go", 0), command("ls", 0, "a", 1)]))).toBeUndefined();
  });
});

describe("turn history", () => {
  it("lists one entry per turn, newest first, with title, duration and change count", () => {
    const turns = turnHistory(chatConversationItems(undefined, null, twoTurns()), false);
    expect(turns.map(turn => turn.title)).toEqual(["Add a retry budget to the policy engine. Run the tests.", "Read the policy engine"]);
    const [budget, readTurn] = turns;
    // Two edits of one file count once with their counts summed; a failed
    // edit changed nothing.
    expect(budget.changes).toEqual({ files: 2, additions: 18, deletions: 3 });
    // From the first tool call's start (22s) to the last one's end (30s + 4s).
    expect(budget.durationMs).toBe(12_000);
    expect(budget.harness).toBe("codex");
    expect(budget.live).toBe(false);
    expect(readTurn.changes).toBeNull();
    expect(readTurn.durationMs).toBe(6_000);
  });

  it("holds back the duration and changes of the turn still running", () => {
    const turns = turnHistory(chatConversationItems(undefined, null, twoTurns()), true);
    expect(turns[0]).toMatchObject({ live: true, changes: null, durationMs: undefined });
    expect(turns[1]).toMatchObject({ live: false, durationMs: 6_000 });
  });

  it("has no duration for a turn with no timed tool calls", () => {
    const turns = turnHistory(chatConversationItems(undefined, null, [prompt("hello", 0), reply("hi", 3)]), false);
    expect(turns).toHaveLength(1);
    expect(turns[0].durationMs).toBeUndefined();
    expect(turns[0].changes).toBeNull();
  });

  it("carries the prompt's forest entry so a row can reveal it", () => {
    const entries = [{
      id: "entry-1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2, kind: "user.message",
      payload: { text: "From the forest", itemId: "user-1" }, providerEventId: null, contextVisibility: "eligible" as const, tokenEstimate: null, createdAt: at(0),
    }];
    const turns = turnHistory(chatConversationItems(entries, "entry-1", []), false);
    expect(turns[0]).toMatchObject({ title: "From the forest", entryId: "entry-1" });
  });

  it("skips rows that came before the first prompt", () => {
    expect(turnHistory(chatConversationItems(undefined, null, [reply("Session restored", 0)]), false)).toEqual([]);
  });
});

describe("turn changes", () => {
  it("adds a multi-file summary's files, and its counts only when no file had a call of its own", () => {
    const summary = (paths: string[], additions: number, seconds: number) =>
      event("diff.updated", { createdAt: at(seconds), data: { path: paths[0], paths, additions, deletions: 0 } });
    const overlapping = chatConversationItems(undefined, null, [prompt("go", 0), edit("a.ts", 2, 1, 1), summary(["a.ts", "b.ts"], 9, 2)]);
    expect(turnChanges(overlapping)).toEqual({ files: 2, additions: 2, deletions: 1 });
    const alone = chatConversationItems(undefined, null, [prompt("go", 0), summary(["a.ts", "b.ts"], 9, 2)]);
    expect(turnChanges(alone)).toEqual({ files: 2, additions: 9, deletions: 0 });
  });

  it("counts files without inventing line counts nobody reported", () => {
    const items = chatConversationItems(undefined, null, [prompt("go", 0), edit("a.ts", undefined, undefined, 1)]);
    expect(turnChanges(items)).toEqual({ files: 1, additions: undefined, deletions: undefined });
  });

  it("ignores reads", () => {
    expect(turnChanges(chatConversationItems(undefined, null, [prompt("go", 0), read("a.ts", 1)]))).toBeNull();
  });
});

describe("wording", () => {
  it("titles a turn by its prompt's first line", () => {
    expect(turnTitle("\n  Fix the build\nthen run tests")).toBe("Fix the build");
    expect(turnTitle("   ")).toBe("Message with no text");
  });

  it("formats durations the way the transcript does", () => {
    expect(formatTurnDuration(400)).toBe("1s");
    expect(formatTurnDuration(34_000)).toBe("34s");
    expect(formatTurnDuration(61_000)).toBe("1m 01s");
    expect(formatTurnDuration(3_725_000)).toBe("1h 02m");
  });
});
