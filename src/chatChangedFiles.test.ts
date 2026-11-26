import { describe, expect, it } from "vitest";
import { chatChangedFiles } from "./chatChangedFiles";
import { reduceConversation } from "./conversation";
import { asWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";

const ROOT = "/Users/you/bridge/Kyoto";

let seq = 0;
function event(kind: string, fields: Partial<AgentEvent> = {}): AgentEvent {
  seq += 1;
  return {
    id: seq, sessionId: "chat", sequence: seq, protocolVersion: 1, kind: asWireKind(kind), itemId: `item-${seq}`,
    role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...fields,
  };
}

/** A Claude tool call, as the live stream carries it. */
function claudeTool(name: string, input: Record<string, unknown>, fields: Partial<AgentEvent> = {}): AgentEvent {
  return event("tool.completed", { title: name, ...fields, data: { type: "tool_use", name, input, ...fields.data } });
}

function changed(events: AgentEvent[], roots: (string | null | undefined)[] = [ROOT]) {
  return Object.fromEntries(chatChangedFiles(reduceConversation(events), roots));
}

describe("chatChangedFiles", () => {
  it("marks an edited file M, keyed by its path inside the chat's folder", () => {
    expect(changed([claudeTool("Edit", { file_path: `${ROOT}/src-tauri/bridge-core/src/policy.rs` })]))
      .toEqual({ "src-tauri/bridge-core/src/policy.rs": "M" });
  });

  it("marks a written file A, unless the harness says it replaced one", () => {
    expect(changed([
      claudeTool("Write", { file_path: `${ROOT}/docs/new.md` }, { data: { aggregatedOutput: `File created successfully at: ${ROOT}/docs/new.md` } }),
      claudeTool("Write", { file_path: `${ROOT}/README.md` }, { data: { aggregatedOutput: `The file ${ROOT}/README.md has been updated successfully.` } }),
    ])).toEqual({ "docs/new.md": "A", "README.md": "M" });
  });

  it("lets the first write decide: a created file edited again stays A", () => {
    expect(changed([
      claudeTool("Write", { file_path: `${ROOT}/src/new.ts` }),
      claudeTool("Edit", { file_path: `${ROOT}/src/new.ts` }),
    ])).toEqual({ "src/new.ts": "A" });
  });

  it("reads every file of a Codex change with its own kind", () => {
    expect(changed([event("file_change.completed", { data: { type: "fileChange", changes: [
      { path: `${ROOT}/src/lib.rs`, kind: "update", diff: "@@ -1 +1 @@\n-a\n+b" },
      { path: `${ROOT}/NOTES.txt`, kind: { type: "add" }, diff: "hello\n" },
    ] } })])).toEqual({ "src/lib.rs": "M", "NOTES.txt": "A" });
  });

  it("takes relative paths as already relative to the chat's folder", () => {
    expect(changed([event("file_change.completed", { title: "tokenStore.ts", data: { path: "src/auth/tokenStore.ts", additions: 9, deletions: 4 } })], []))
      .toEqual({ "src/auth/tokenStore.ts": "M" });
    expect(changed([claudeTool("Edit", { file_path: "./src/App.tsx" })], [])).toEqual({ "src/App.tsx": "M" });
  });

  it("marks a patch that creates its file A", () => {
    const patch = "diff --git a/src/fresh.ts b/src/fresh.ts\nnew file mode 100644\n--- /dev/null\n+++ b/src/fresh.ts\n@@ -0,0 +1 @@\n+export {};";
    expect(changed([event("file_change.completed", { data: { path: "src/fresh.ts", patch } })], [])).toEqual({ "src/fresh.ts": "A" });
  });

  it("ignores reads, searches, commands, failed edits and edits still being composed", () => {
    expect(changed([
      claudeTool("Read", { file_path: `${ROOT}/src/read.ts` }),
      claudeTool("Grep", { pattern: "policy", path: `${ROOT}/src` }),
      claudeTool("Bash", { command: "touch src/touched.ts" }),
      claudeTool("Edit", { file_path: `${ROOT}/src/failed.ts` }, { status: "failed" }),
      claudeTool("Edit", { file_path: `${ROOT}/src/preparing.ts` }, { status: "inProgress", data: { phase: "preparing" } }),
    ])).toEqual({});
  });

  it("forgets a file the chat deleted", () => {
    expect(changed([
      claudeTool("Edit", { file_path: `${ROOT}/src/gone.ts` }),
      event("tool.completed", { title: "Delete gone.ts", data: { kind: "delete", locations: [{ path: `${ROOT}/src/gone.ts` }] } }),
    ])).toEqual({});
  });

  it("drops a path outside every root, and prefers the longest root", () => {
    const worktree = `${ROOT}/.worktrees/task`;
    expect(changed([
      claudeTool("Edit", { file_path: "/etc/hosts" }),
      claudeTool("Edit", { file_path: `${worktree}/src/a.ts` }),
    ], [ROOT, `${worktree}/`])).toEqual({ "src/a.ts": "M" });
  });

  it("ignores roots that are missing or not absolute", () => {
    expect(changed([claudeTool("Edit", { file_path: `${ROOT}/src/a.ts` })], [null, undefined, "relative/root"])).toEqual({});
  });
});
