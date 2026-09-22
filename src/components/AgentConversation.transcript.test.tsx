// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import { durableEntriesFrom } from "../transcript/golden";
import type { AgentEvent, Session, SessionEntry } from "../types";

// The three-layer tool card: what a row shows at a glance, what it opens into,
// and what it folds away. Motion is skipped so these are assertions about the
// rendering, not about a frame.

const session: Session = {
  id: "s", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working",
  startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null,
  metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh",
  continuationFidelity: "native", kind: "orchestrator",
} as Session;

const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
  role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {},
  createdAt: "now", ...overrides,
});

const forestEntry = (id: string, parentEntryId: string | null, sequence: number, kind: string, payload: Record<string, unknown>): SessionEntry => ({
  id, sessionId: "s", parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload,
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
});

const TWO_HUNKS = [
  "@@ -118,3 +118,3 @@ impl Runtime {",
  "-    let state = self.store.lock().session_state(id)?;",
  "+    let state = self.store.lock_scoped(|db| db.session_state(id))?;",
  "@@ -204,2 +204,3 @@ impl Reader {",
  "+        if self.generation != current_generation() { return; }",
].join("\n");

const fileChange = (overrides: Partial<AgentEvent> = {}) => event(1, "file_change.completed", {
  title: "lib.rs",
  data: { path: "src-tauri/src/lib.rs", additions: 24, deletions: 3, patch: TWO_HUNKS },
  ...overrides,
});

let host: HTMLDivElement;
let root: Root;

function mount(events: AgentEvent[]) {
  act(() => {
    root.render(<AgentConversation session={session} events={events} onResolve={() => {}} />);
  });
}

/** The chip itself, not the meta cell that happens to contain only the chip. */
const exitChip = (label: string) =>
  [...host.querySelectorAll<HTMLElement>("span.rounded-full")].find(node => node.textContent === label);

const buttonWith = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(text));

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

describe("anonymous tool starts", () => {
  // Adapter-shaped regression data, not a capture of the original report.
  const start = () => event(1, "tool.started", {
    itemId: "context", title: "", status: "inProgress",
    data: { kind: "other", update: { sessionUpdate: "tool_call", toolCallId: "context", title: "", kind: "other", status: "in_progress" } },
  });

  function mountProjection(events: AgentEvent[], durable: boolean) {
    const entries = durable ? durableEntriesFrom("s", events) : undefined;
    act(() => root.render(<AgentConversation session={session} events={durable ? [] : events} forestEntries={entries} activeLeafId={entries?.at(-1)?.id} onResolve={() => {}} />));
  }

  it.each([false, true])("omits an empty pending call without leaving a tool group (replay=%s)", (durable) => {
    mountProjection([start()], durable);
    expect(host.textContent).not.toContain("Using a tool");
    expect(host.querySelector("[data-activity-group]")).toBeNull();
  });

  it("reveals the same call when a progress update supplies its action", () => {
    const started = start();
    mount([started]);
    expect(host.querySelector("[data-activity-group]")).toBeNull();
    mount([started, event(0, "tool.progress", {
      sequence: 0, itemId: "context", title: "Resolve project context", status: "inProgress",
      data: { sessionUpdate: "tool_call_update", toolCallId: "context", title: "Resolve project context", status: "in_progress" },
    })]);
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    expect(host.textContent).toContain("Running: Resolve project context");
    expect(host.textContent).not.toContain("Using a tool");
  });

  it("does not count an anonymous placeholder alongside a named tool", () => {
    mount([start(), event(2, "tool.started", {
      itemId: "read", title: "project", status: "inProgress", data: { kind: "read" },
    })]);
    expect(host.textContent).toContain("Reading project");
    expect(buttonWith("step")?.textContent).toContain("1 step");
    expect(host.textContent).not.toContain("Using a tool");
  });

  it("reveals anonymous work as soon as actual output arrives", () => {
    mount([start(), event(0, "tool.progress", {
      sequence: 0, itemId: "context", status: "inProgress", text: "Context loaded",
    })]);
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    act(() => buttonWith("step")!.click());
    act(() => host.querySelector<HTMLButtonElement>('[aria-label="Expand tool output"]')!.click());
    expect(host.textContent).toContain("Context loaded");
  });

  it.each([
    ["completed", false], ["completed", true], ["failed", false], ["failed", true],
  ] as const)("retains an anonymous %s result (replay=%s)", (status, durable) => {
    const events = [start(), event(2, "tool.completed", { itemId: "context", status })];
    mountProjection(events, durable);
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    act(() => buttonWith("step")!.click());
    expect(host.textContent).toContain("Used a tool");
  });
});

describe("inline diffs", () => {
  it("shows an edit's first hunk without anyone clicking anything", () => {
    mount([fileChange()]);
    expect(host.textContent).toContain("lock_scoped");
    expect(host.querySelector(".stx")).not.toBeNull();
  });

  it("folds the remaining hunks behind a bar rather than truncating the patch", () => {
    mount([fileChange()]);
    expect(host.textContent).not.toContain("current_generation");
    const fold = buttonWith("more hunk");
    expect(fold?.textContent).toMatch(/1 more hunk\b.*expand/);

    act(() => fold!.click());
    expect(host.textContent).toContain("current_generation");
  });

  it("keeps the diffstat and full path discoverable on the summary row", () => {
    mount([fileChange()]);
    expect(host.textContent).toContain("+24");
    expect(host.textContent).toContain("−3");
    expect(host.querySelector('[title="src-tauri/src/lib.rs"]')?.textContent).toBe("src-tauri/src");
  });

  it("still opens a group whose edit carries no diff only on request", () => {
    mount([event(1, "file_change.completed", { title: "lib.rs", data: { path: "src-tauri/src/lib.rs", additions: 1, deletions: 0 } })]);
    // Nothing to show inline, so the group stays folded the way it always did.
    expect(host.textContent).not.toContain("src-tauri/src/lib.rs");
    expect(buttonWith("Edited 1 file")).toBeDefined();
  });
});

describe("command rows", () => {
  const command = (data: Record<string, unknown>, overrides: Partial<AgentEvent> = {}) =>
    event(1, "command.completed", {
      title: "bun run test",
      data: { type: "commandExecution", command: "bun run test", aggregatedOutput: "92 pass\n0 fail", ...data },
      ...overrides,
    });

  async function openGroup(events: AgentEvent[]) {
    mount(events);
    act(() => buttonWith("Ran 1 command")!.click());
  }

  it("renders a zero exit code as a success chip", async () => {
    await openGroup([command({ exitCode: 0 })]);
    const chip = exitChip("exit 0");
    expect(chip).toBeDefined();
    expect(chip?.className).toContain("text-success");
  });

  it("renders a nonzero exit code in the destructive tone", async () => {
    await openGroup([command({ exitCode: 2 })]);
    const chip = exitChip("exit 2");
    expect(chip).toBeDefined();
    expect(chip?.className).toContain("text-destructive");
    expect(buttonWith("Activity needs attention")).toBeDefined();
  });

  it("renders no chip at all when the provider reports no exit code", async () => {
    await openGroup([command({})]);
    expect(host.textContent).not.toMatch(/exit \S/);
  });

  it("expands into a terminal block with a prompt line and dimmed output", async () => {
    await openGroup([command({ exitCode: 0 })]);
    act(() => buttonWith("Ran bun run test")!.click());
    expect(host.textContent).toContain("❯");
    expect(host.textContent).toContain("92 pass");
  });
});

describe("three layers", () => {
  const read = (id: number, path: string) => event(id, "tool.completed", {
    title: `Read ${path}`, data: { type: "readFile", path },
  });

  it("keeps reads, searches, and edits in one activity section", () => {
    mount([
      read(1, "src-tauri/src/lib.rs"),
      event(2, "tool.completed", { title: "grep", data: { name: "Grep", input: { pattern: "resume" } } }),
      fileChange({ id: 3, sequence: 3, itemId: "i-3" }),
    ]);
    // The group is already open, because the edit carries a diff.
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    expect(host.textContent).not.toContain("Explored");
  });

  it("puts each action in the shared activity section with the patch still visible", () => {
    mount([read(1, "src-tauri/src/lib.rs"), fileChange({ id: 2, sequence: 2, itemId: "i-2" })]);
    const activity = host.querySelector("[data-activity-group]");
    expect(activity?.querySelectorAll("[data-tool-row]")).toHaveLength(2);
    expect(buttonWith("Edited lib.rs")?.closest("[data-activity-group]")).toBe(activity);
    expect(buttonWith("Read lib.rs")?.closest("[data-activity-group]")).toBe(activity);
    expect(host.querySelector(".stx")).not.toBeNull();
    // The basename appears once per action, with the parent path as context.
    expect(buttonWith("Read lib.rs")?.closest("[data-tool-row]")?.textContent).toBe("Read lib.rssrc-tauri/src");
  });

  it("does not reorder the transcript to tidy it", () => {
    mount([
      read(1, "a.rs"),
      fileChange({ id: 2, sequence: 2, itemId: "i-2" }),
      read(3, "b.rs"),
    ]);
    const text = host.textContent ?? "";
    expect(text.indexOf("Read a.rs")).toBeLessThan(text.indexOf("Edited lib.rs"));
    expect(text.indexOf("Edited lib.rs")).toBeLessThan(text.indexOf("Read b.rs"));
  });

  it("keeps a plan in stream order without splitting the run around it", () => {
    // A plan update is the model narrating work in progress, so it travels
    // with the run rather than cutting it in two. It keeps its place in the
    // timeline: between the command before it and the command after it.
    mount([
      event(1, "command.completed", { title: "bun test", data: { type: "commandExecution", command: "bun test" } }),
      event(2, "plan.updated", { title: "Next step", data: { steps: [{ step: "Run tests", status: "completed" }] } }),
      event(3, "command.completed", { title: "bun run check", data: { type: "commandExecution", command: "bun run check" } }),
    ]);
    expect([...host.querySelectorAll("button")].filter(btn => btn.textContent?.includes("Ran 2 commands"))).toHaveLength(1);
    act(() => buttonWith("Ran 2 commands")!.click());
    const text = host.textContent ?? "";
    expect(text.indexOf("Ran bun test")).toBeLessThan(text.indexOf("Next step"));
    expect(text.indexOf("Next step")).toBeLessThan(text.indexOf("Ran bun run check"));
  });

  it("preserves multiple distinct plan items without dropping", () => {
    mount([
      event(1, "command.completed", { title: "bun test", data: { type: "commandExecution", command: "bun test" } }),
      event(2, "plan.updated", { itemId: "plan-1", title: "Plan Phase 1", data: { steps: [{ step: "Phase 1", status: "completed" }] } }),
      event(3, "plan.updated", { itemId: "plan-2", title: "Plan Phase 2", data: { steps: [{ step: "Phase 2", status: "inProgress" }] } }),
      event(4, "command.completed", { title: "bun run check", data: { type: "commandExecution", command: "bun run check" } }),
    ]);
    act(() => buttonWith("Ran 2 commands")!.click());
    expect(host.textContent).toContain("Plan Phase 1");
    expect(host.textContent).toContain("Plan Phase 2");
  });

  it("preserves reasoning order when a thought occurs after commands", () => {
    mount([
      event(1, "command.completed", { title: "bun test", data: { type: "commandExecution", command: "bun test" } }),
      event(2, "reasoning.completed", { itemId: "r-after", text: "Post-execution thought reflection", status: "completed" }),
    ]);
    const fullText = host.textContent ?? "";
    const commandIndex = fullText.indexOf("Ran 1 command");
    const reasoningIndex = fullText.indexOf("Thought for a moment");
    expect(commandIndex).toBeGreaterThan(-1);
    expect(reasoningIndex).toBeGreaterThan(commandIndex);
    expect(fullText).toContain("Post-execution thought reflection");
  });

  it("keeps exploratory CLI commands in the same chronological activity list", () => {
    mount([
      event(1, "command.completed", { title: "cat src/auth.rs", data: { type: "commandExecution", command: "cat src/auth.rs" } }),
      event(2, "command.completed", { title: "git status", data: { type: "commandExecution", command: "git status" } }),
    ]);
    expect(host.textContent).toContain("Read 2 files");
    act(() => buttonWith("Read 2 files")!.click());
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    expect(host.textContent).toContain("Read auth.rs");
    expect(host.textContent).toContain("Checked git status");
    expect(host.querySelectorAll("[data-tool-row]")).toHaveLength(2);
  });

  it("interleaves live-only rows into durable history by causal anchor", () => {
    // Streamed frames carry sequence 0 until persisted. The reply that
    // followed a durably sequenced tool card must render below it — not
    // hoisted above the turn, and not pinned under later durable rows.
    const entries: SessionEntry[] = [
      forestEntry("e1", null, 1, "user.message", { text: "where does it live?", role: "user" }),
      forestEntry("e2", "e1", 2, "command.started", { itemId: "t1", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
      forestEntry("e3", "e2", 3, "command.completed", { itemId: "t1", title: "bun test", status: "completed", data: { type: "commandExecution", command: "bun test" } }),
    ];
    const live = [
      event(2, "command.started", { itemId: "t1", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
      event(3, "command.completed", { itemId: "t1", title: "bun test", status: "completed", data: { type: "commandExecution", command: "bun test" } }),
      event(0, "message.delta", { sequence: 0, itemId: "m1", role: "assistant", status: "streaming", text: "Found it in the session store." }),
    ];
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={entries} activeLeafId="e3" onResolve={() => {}} />);
    });
    const text = host.textContent ?? "";
    expect(text.indexOf("Ran 1 command")).toBeGreaterThan(text.indexOf("where does it live?"));
    expect(text.indexOf("Found it in the session store.")).toBeGreaterThan(text.indexOf("Ran 1 command"));
  });

  it("renders an unnamed first reply once when the forest and live stream both carry it", () => {
    const reply = "Hi — what would you like to work on in Bridge?";
    const entries: SessionEntry[] = [
      forestEntry("e1", null, 1, "user.message", { itemId: "u1", text: "hi", role: "user" }),
      forestEntry("e2", "e1", 2, "assistant.message", { itemId: "acp-message-1", text: reply, role: "assistant", status: "completed" }),
    ];
    const live = [
      event(1, "message.completed", { itemId: "u1", role: "user", status: "completed", text: "hi" }),
      event(0, "message.delta", { sequence: 0, itemId: null, role: "assistant", status: "streaming", text: reply }),
      event(2, "message.completed", { itemId: "acp-message-1", role: "assistant", status: "completed", text: reply }),
    ];
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={entries} activeLeafId="e2" onResolve={() => {}} />);
    });
    const occurrences = host.textContent?.split(reply).length ?? 0;
    expect(occurrences).toBe(2);
  });

  it("keeps a reply that streamed before its tools above their cards after it persists", () => {
    // The forest sequences an assistant message at completion time — after
    // tool entries it causally preceded. The live window watched the text
    // stream first, so the merged row takes that earlier anchor.
    const entries: SessionEntry[] = [
      forestEntry("e1", null, 1, "user.message", { itemId: "u1", text: "where does it live?", role: "user" }),
      forestEntry("e2", "e1", 2, "command.started", { itemId: "t1", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
      forestEntry("e3", "e2", 3, "command.completed", { itemId: "t1", title: "bun test", status: "completed", data: { type: "commandExecution", command: "bun test" } }),
      forestEntry("e4", "e3", 4, "assistant.message", { itemId: "m1", text: "Let me look at the store first.", role: "assistant", status: "completed" }),
    ];
    const live = [
      event(1, "message.completed", { itemId: "u1", role: "user", status: "completed", text: "where does it live?" }),
      event(0, "message.delta", { sequence: 0, itemId: "m1", role: "assistant", status: "streaming", text: "Let me look at the store first." }),
      event(2, "command.started", { itemId: "t1", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
      event(3, "command.completed", { itemId: "t1", title: "bun test", status: "completed", data: { type: "commandExecution", command: "bun test" } }),
      event(4, "message.completed", { itemId: "m1", role: "assistant", status: "completed", text: "Let me look at the store first." }),
    ];
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={entries} activeLeafId="e4" onResolve={() => {}} />);
    });
    const text = host.textContent ?? "";
    expect(text.indexOf("Let me look at the store first.")).toBeGreaterThan(text.indexOf("where does it live?"));
    expect(text.indexOf("Ran 1 command")).toBeGreaterThan(text.indexOf("Let me look at the store first."));
  });

  it("renders replayed forest reasoning as collapsible Thought for a moment", () => {
    const reasoningEntry: SessionEntry = {
      id: "r1",
      sessionId: "s",
      parentEntryId: null,
      sequence: 1,
      semanticSchemaVersion: 2,
      kind: "reasoning.completed",
      payload: { text: "Thinking deeply about architecture", status: "completed" },
      providerEventId: null,
      contextVisibility: "eligible" as const,
      tokenEstimate: null,
      createdAt: "now",
    };
    act(() => {
      root.render(<AgentConversation session={session} events={[]} forestEntries={[reasoningEntry]} activeLeafId="r1" onResolve={() => {}} />);
    });
    expect(host.textContent).toContain("Thought for a moment");
    expect(host.textContent).toContain("Thinking deeply about architecture");
    expect(host.textContent).not.toContain("Reasoning completed");
  });
});

describe("run trailer", () => {
  const parallelCommand = (id: number, at: string) =>
    event(id, "command.completed", {
      title: `cargo test ${id}`,
      createdAt: at,
      data: { type: "commandExecution", command: `cargo test ${id}`, durationMs: 10000, aggregatedOutput: "ok", exitCode: 0 },
    });

  it("reports the run's wall-clock span, not the summed duration of overlapping calls", () => {
    // Two calls run in parallel: same start, 10s each. The trailer must read the
    // union of their windows (~10s), never the doubled sum (20s).
    mount([parallelCommand(1, "2026-01-01T00:00:00.000Z"), parallelCommand(2, "2026-01-01T00:00:00.000Z")]);
    expect(host.textContent).toContain("Worked for 10s");
    expect(host.textContent).not.toContain("20s");
  });
});

describe("harness subagents (issue #667)", () => {
  it.each([
    ["collabAgentToolCall", false], ["collabAgentToolCall", true],
    ["dynamicToolCall", false], ["dynamicToolCall", true],
  ] as const)("keeps a title-less %s prompt and child lifecycle inspectable (replay=%s)", async (type, durable) => {
    const started = event(1, "tool.started", {
      itemId: "child-call", status: "inProgress", title: null,
      data: {
        type,
        ...(type === "collabAgentToolCall"
          ? { prompt: "Map the login flow" }
          : { arguments: { prompt: "Map the login flow", subagent_type: "Explore" } }),
        threadId: "child", agentsStates: { child: { status: "inProgress" } },
      },
    });
    const mountProjection = async (events: AgentEvent[]) => {
      const entries = durable ? durableEntriesFrom("s", events) : undefined;
      await act(async () => root.render(<AgentConversation session={session} events={durable ? [] : events} forestEntries={entries} activeLeafId={entries?.at(-1)?.id} onResolve={() => {}} />));
    };
    await mountProjection([started]);
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    await act(async () => buttonWith("Using 1 tool")!.click());
    await act(async () => buttonWith("Using a tool")!.click());
    expect(host.textContent).toContain("Map the login flow");
    expect(host.textContent).toContain("Running subagent");

    await mountProjection([started, event(2, "tool.completed", {
      itemId: "child-call", status: "completed", title: null,
      data: { agentsStates: { child: { status: "completed", message: "Found three call sites." } } },
    })]);
    expect(host.textContent).toContain("Map the login flow");
    expect(host.textContent).toContain("Subagent finished");
    expect(host.textContent).toContain("Found three call sites.");
    expect(host.textContent).not.toContain("Running subagent");
  });

  const subagentDone = () => event(1, "tool.completed", {
    itemId: "task-1",
    title: "Task",
    text: "Auth lives in src/auth.ts with a session cookie.",
    data: {
      name: "Task",
      input: { description: "Explore auth", prompt: "Map the login flow", subagent_type: "Explore" },
    },
  });

  async function openSubagentRow(events: AgentEvent[]) {
    mount(events);
    act(() => buttonWith("Used 1 tool")!.click());
    act(() => buttonWith("Delegated Explore auth")!.click());
  }

  it("opens into the prompt that was sent and the result that came back", async () => {
    await openSubagentRow([subagentDone()]);
    expect(host.textContent).toContain("Subagent finished");
    expect(host.textContent).toContain("Explore");
    expect(host.textContent).toContain("Map the login flow");
    expect(host.textContent).toContain("Auth lives in src/auth.ts");
  });

  it("shows the prompt while the subagent is still running", async () => {
    mount([event(1, "tool.started", {
      itemId: "task-1",
      title: "Task",
      status: "inProgress",
      data: {
        name: "Task",
        input: { description: "Explore auth", prompt: "Map the login flow", subagent_type: "Explore" },
      },
    })]);
    act(() => buttonWith("Using 1 tool")!.click());
    act(() => buttonWith("Delegating Explore auth")!.click());
    expect(host.textContent).toContain("Map the login flow");
    expect(host.textContent).toContain("the result will appear here");
  });

  it("leaves ordinary tool rows exactly as before", async () => {
    mount([event(1, "tool.completed", {
      title: "Read",
      data: { name: "Read", input: { file_path: "src/lib.rs" } },
      text: "fn a() {}\n",
    })]);
    expect(host.textContent).not.toContain("Subagent");
    expect(host.textContent).not.toContain("Asked");
  });

  it("shows a running child even when the parent tool call is completed", async () => {
    mount([event(1, "tool.completed", {
      itemId: "task-1",
      title: "Task",
      status: "completed",
      data: {
        name: "Task",
        input: { description: "Explore auth", prompt: "Map the login flow" },
        threadId: "t-child",
        agentsStates: { "t-child": { status: "inProgress" } },
      },
    })]);
    act(() => buttonWith("Used 1 tool")!.click());
    act(() => buttonWith("Delegated Explore auth")!.click());
    expect(host.textContent).toContain("Running subagent");
  });

  it("shows a failed child status instead of a green check", async () => {
    mount([event(1, "tool.completed", {
      itemId: "task-1",
      title: "Task",
      status: "completed",
      data: {
        name: "Task",
        input: { description: "Explore auth", prompt: "Map the login flow" },
        threadId: "t-child",
        agentsStates: { "t-child": { status: "failed" } },
      },
    })]);
    act(() => buttonWith("Used 1 tool")!.click());
    act(() => buttonWith("Delegated Explore auth")!.click());
    expect(host.textContent).toContain("Subagent failed");
    expect(host.textContent).not.toContain("Subagent finished");
  });
});
