import { describe, expect, it } from "vitest";
import { alignTurns, groupItems, reduceConversation, type ConversationItem } from "./conversation";
import { asWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";
import { changeTree, describeTurnEnds, diffstat, formatWorked, keepStable, splitPatchByFile, turnChangesByTurn, turnEndKeys, turnModels, workedMs } from "./turnChanges";

let id = 0;
const event = (kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => {
  id += 1;
  return {
    id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
    role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {},
    createdAt: "2026-11-20T18:17:00.000Z", ...overrides,
  };
};
const ask = (text: string, overrides: Partial<AgentEvent> = {}) => event("message.completed", { role: "user", text, ...overrides });
const reply = (text: string, overrides: Partial<AgentEvent> = {}) => event("message.completed", { role: "assistant", text, ...overrides });
const edit = (path: string, data: Record<string, unknown> = {}, overrides: Partial<AgentEvent> = {}) =>
  event("file_change.completed", { title: path.split("/").at(-1), data: { path, ...data }, ...overrides });
const read = (path: string, overrides: Partial<AgentEvent> = {}) =>
  event("tool.completed", { title: `Read ${path}`, data: { type: "readFile", path }, ...overrides });
const items = (events: AgentEvent[]): ConversationItem[] => alignTurns(reduceConversation(events));

const MODIFY = "--- a/src/policy.rs\n+++ b/src/policy.rs\n@@ -1,2 +1,3 @@\n fn a() {}\n-fn b() {}\n+fn b() { 1 }\n+fn c() {}";
const CREATE = "--- /dev/null\n+++ b/testing/retry.md\n@@ -0,0 +1,2 @@\n+# Retry\n+budget";

describe("turnChangesByTurn", () => {
  it("folds a file edited twice in one turn into one entry with summed counts", () => {
    const turns = turnChangesByTurn(items([
      ask("Add a retry budget"),
      edit("src/policy.rs", { additions: 2, deletions: 1, patch: MODIFY }),
      edit("src/policy.rs", { additions: 5, deletions: 0 }),
      edit("src/coordinator.rs", { additions: 3, deletions: 2 }),
    ]));
    const turn = turns.get(1)!;
    expect(turn.files.map(file => [file.path, file.mark, file.additions, file.deletions])).toEqual([
      ["src/policy.rs", "M", 7, 1],
      ["src/coordinator.rs", "M", 3, 2],
    ]);
    expect(turn.files[0].patches).toEqual([MODIFY]);
    expect([turn.additions, turn.deletions]).toEqual([10, 3]);
  });

  it("marks a file the patch creates as added and everything else as modified", () => {
    const turn = turnChangesByTurn(items([
      ask("go"),
      edit("testing/retry.md", { patch: CREATE }),
      edit("src/policy.rs", { patch: MODIFY }),
      edit("src/unknown.rs", { additions: 1, deletions: 0 }),
    ])).get(1)!;
    expect(turn.files.map(file => [file.path, file.mark])).toEqual([
      ["testing/retry.md", "A"],
      ["src/policy.rs", "M"],
      ["src/unknown.rs", "M"],
    ]);
    // Counts come off the patch when the call left them out.
    expect([turn.files[0].additions, turn.files[0].deletions]).toEqual([2, 0]);
    expect([turn.files[1].additions, turn.files[1].deletions]).toEqual([2, 1]);
  });

  it("reads a creation over a file the chat already saw as a rewrite", () => {
    // Claude reports every Write as a creation, existing file or not.
    const rewrite = "--- /dev/null\n+++ b/src/policy.rs\n@@ -0,0 +1,1 @@\n+fn a() {}";
    const turns = turnChangesByTurn(items([
      ask("look"),
      read("src/policy.rs"),
      reply("Read it."),
      ask("rewrite it"),
      edit("src/policy.rs", { patch: rewrite }),
      edit("src/fresh.rs", { patch: rewrite.replaceAll("src/policy.rs", "src/fresh.rs") }),
      edit("src/fresh.rs", { additions: 1, deletions: 1 }),
    ]));
    expect(turns.get(1)).toBeUndefined();
    expect(turns.get(2)!.files.map(file => [file.path, file.mark])).toEqual([
      ["src/policy.rs", "M"],
      // Created this turn stays created through a later edit in the same turn.
      ["src/fresh.rs", "A"],
    ]);
  });

  it("ignores reads, failed edits and pathless turn snapshots", () => {
    const turns = turnChangesByTurn(items([
      ask("go"),
      read("src/a.rs"),
      edit("src/b.rs", { additions: 4, deletions: 0 }, { status: "failed" }),
      // Codex's turn diff: the whole turn again, with no file of its own.
      event("diff.updated", { itemId: null, data: { turnId: "t1", diff: "diff --git a/src/c.rs b/src/c.rs\n--- a/src/c.rs\n+++ b/src/c.rs\n@@ -1 +1 @@\n-a\n+b" } }),
      edit("src/c.rs", { additions: 1, deletions: 1 }),
    ]));
    expect(turns.get(1)!.files.map(file => file.path)).toEqual(["src/c.rs"]);
    expect([turns.get(1)!.additions, turns.get(1)!.deletions]).toEqual([1, 1]);
  });

  it("gives a turn without edits no entry at all", () => {
    expect(turnChangesByTurn(items([ask("hi"), read("src/a.rs"), reply("Hello.")])).size).toBe(0);
    expect(turnChangesByTurn([]).size).toBe(0);
  });

  it("splits a Codex change list per file, with its kind", () => {
    const turn = turnChangesByTurn(items([
      ask("go"),
      edit("src/lib.rs", {
        type: "fileChange",
        paths: ["src/lib.rs", "NOTES.txt"],
        changes: [
          { path: "src/lib.rs", kind: { type: "update" }, diff: "@@ -1,1 +1,1 @@\n-fn a() {}\n+fn a() { 1 }\n" },
          { path: "NOTES.txt", kind: "add", diff: "one\ntwo\n" },
        ],
      }),
    ])).get(1)!;
    expect(turn.files.map(file => [file.path, file.mark, file.additions, file.deletions])).toEqual([
      ["src/lib.rs", "M", 1, 1],
      ["NOTES.txt", "A", 2, 0],
    ]);
    expect(turn.files[1].patches[0]).toContain("--- /dev/null");
  });

  it("splits a joined multi-file patch at its file headers", () => {
    const joined = [
      "diff --git a/src/a.ts b/src/a.ts",
      "--- a/src/a.ts",
      "+++ b/src/a.ts",
      "@@ -1,2 +1,2 @@",
      "--- a comment that starts like a header",
      "+++ and its replacement",
      " kept",
      "--- /dev/null",
      "+++ b/src/b.ts",
      "@@ -0,0 +1,1 @@",
      "+export {};",
    ].join("\n");
    expect(splitPatchByFile(joined)?.map(file => [file.path, file.mark, file.additions, file.deletions])).toEqual([
      ["src/a.ts", undefined, 1, 1],
      ["src/b.ts", "A", 1, 0],
    ]);
    const applyPatch = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-fn a() {}\n+fn a() { 1 }\n*** Add File: src/new.rs\n+pub fn n() {}\n*** End Patch\n";
    expect(splitPatchByFile(applyPatch)?.map(file => [file.path, file.mark, file.additions, file.deletions])).toEqual([
      ["src/lib.rs", "M", 1, 1],
      ["src/new.rs", "A", 1, 0],
    ]);
    expect(splitPatchByFile("@@ -1 +1 @@\n-a\n+b")).toBeUndefined();
  });

  it("names every file of a call it cannot split, and keeps its counts in the total only", () => {
    const turn = turnChangesByTurn(items([
      ask("go"),
      edit("src/a.rs", { paths: ["src/a.rs", "src/b.rs"], additions: 6, deletions: 2 }),
    ])).get(1)!;
    expect(turn.files.map(file => [file.path, file.additions])).toEqual([["src/a.rs", undefined], ["src/b.rs", undefined]]);
    expect([turn.additions, turn.deletions]).toEqual([6, 2]);
  });

  it("shows paths relative to the chat's working directory", () => {
    const turn = turnChangesByTurn(items([
      ask("go"),
      edit("/Users/you/repo/src/a.rs", { additions: 1, deletions: 0 }),
      edit("/tmp/scratch.py", { additions: 2, deletions: 0 }),
    ]), "/Users/you/repo").get(1)!;
    expect(turn.files.map(file => file.path)).toEqual(["src/a.rs", "/tmp/scratch.py"]);
  });
});

describe("diffstat", () => {
  it("counts body lines and skips every header", () => {
    expect(diffstat(MODIFY)).toEqual({ additions: 2, deletions: 1 });
    expect(diffstat("diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file")).toEqual({ additions: 1, deletions: 1 });
  });
});

describe("changeTree", () => {
  it("groups files by folder, sorted, with loose files last", () => {
    const file = (path: string) => ({ path, mark: "M" as const, additions: 1, deletions: 0, patches: [] });
    const tree = changeTree([file("testing/a.md"), file("README.md"), file("src/components/Card.tsx"), file("src-tauri/core/policy.rs"), file("src-tauri/core/coordinator.rs")]);
    expect(tree.map(group => [group.folder, group.files.map(entry => entry.path)])).toEqual([
      ["src-tauri/core/", ["src-tauri/core/coordinator.rs", "src-tauri/core/policy.rs"]],
      ["src/components/", ["src/components/Card.tsx"]],
      ["testing/", ["testing/a.md"]],
      ["", ["README.md"]],
    ]);
  });
});

describe("turnModels", () => {
  it("credits each turn to the model in effect when it opened", () => {
    const list = items([
      ask("one"),
      reply("first"),
      event("session.model_changed", { itemId: "switch", data: { previousModel: "gpt-5.6-luna", model: "opus", previousHarness: "codex", harness: "claude" } }),
      ask("two"),
      reply("second"),
    ]);
    expect(list.some(item => item.type === "model-change")).toBe(true);
    const models = turnModels(list, "opus");
    expect(models.get(1)).toBe("gpt-5.6-luna");
    expect(models.get(2)).toBe("opus");
  });

  it("falls back to the chat's model with no change on record, and to nothing without one", () => {
    const list = items([ask("one"), reply("first")]);
    expect(turnModels(list, "gpt-5.6-luna").get(1)).toBe("gpt-5.6-luna");
    expect(turnModels(list, null).get(1)).toBeUndefined();
  });
});

describe("describeTurnEnds", () => {
  const at = (seconds: number) => new Date(Date.UTC(2026, 10, 20, 18, 17, seconds)).toISOString();
  const turn = [
    ask("Add a retry budget"),
    event("command.completed", { title: "cargo test", createdAt: at(0), data: { type: "commandExecution", command: "cargo test", exitCode: 0, durationMs: 4000 } }),
    edit("src/policy.rs", { additions: 2, deletions: 1, durationMs: 1000 }, { createdAt: at(30) }),
    reply("Done.", { createdAt: at(34) }),
  ];

  it("ends a settled turn on its reply with the changes and a caption", () => {
    const list = items(turn);
    const entries = groupItems(list);
    const ends = describeTurnEnds(entries, list, { sessionModel: "gpt-5.6-luna", now: Date.parse(at(40)) });
    const replyKey = entries.at(-1)!.key;
    expect([...ends.keys()]).toEqual([replyKey]);
    const end = ends.get(replyKey)!;
    expect(end.changes?.files.map(file => file.path)).toEqual(["src/policy.rs"]);
    expect(end.line).toMatch(/^\S.* · 31s · GPT Luna$/);
  });

  it("draws nothing for the turn still running", () => {
    const list = items(turn);
    expect(describeTurnEnds(groupItems(list), list, { liveTurn: 1, sessionModel: "gpt-5.6-luna" }).size).toBe(0);
  });

  it("omits every part of the caption it does not know", () => {
    const list = items([ask("hi", { createdAt: "now" }), reply("Hello.", { createdAt: "now" })]);
    const ends = describeTurnEnds(groupItems(list), list, {});
    // No timestamp, no tool work, no model: nothing to say and nothing changed.
    expect(ends.size).toBe(0);
    const withModel = describeTurnEnds(groupItems(list), list, { sessionModel: "gpt-5.6-luna" });
    expect([...withModel.values()].map(end => [end.line, end.changes])).toEqual([["GPT Luna", null]]);
  });

  it("ends a turn without a reply on its last row, never on bookkeeping or the person's message", () => {
    const list = items([
      ask("go"),
      edit("src/a.rs", { additions: 1, deletions: 0 }),
      event("context.compacted", { itemId: "compact", data: { trigger: "auto" } }),
      ask("only a question"),
    ]);
    const entries = groupItems(list);
    const keys = turnEndKeys(entries);
    const group = entries.find(entry => entry.kind === "group")!;
    expect(keys.get(1)).toBe(group.key);
    expect(keys.has(2)).toBe(false);
  });

  it("keeps the previous object for an end that did not change", () => {
    const list = items(turn);
    const first = describeTurnEnds(groupItems(list), list, { sessionModel: "opus", now: 0 });
    const second = keepStable(first, describeTurnEnds(groupItems(list), list, { sessionModel: "opus", now: 0 }));
    const key = [...first.keys()][0];
    expect(second.get(key)).toBe(first.get(key));
    const moved = keepStable(first, describeTurnEnds(groupItems(list), list, { sessionModel: "gpt-5.6-luna", now: 0 }));
    expect(moved.get(key)).not.toBe(first.get(key));
  });
});

describe("worked duration", () => {
  it("spans the union of the calls, as the Worked for header does", () => {
    const list = items([
      ask("go"),
      event("command.completed", { createdAt: "2026-11-20T18:17:00.000Z", data: { type: "commandExecution", command: "a", durationMs: 10_000 } }),
      event("command.completed", { createdAt: "2026-11-20T18:17:05.000Z", data: { type: "commandExecution", command: "b", durationMs: 2_000 } }),
    ]);
    expect(workedMs(list)).toBe(10_000);
    expect(formatWorked(10_000)).toBe("10s");
    expect(formatWorked(125_000)).toBe("2m 05s");
    expect(formatWorked(3_780_000)).toBe("1h 03m");
  });
});
