// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { SHOW_THINKING_STORAGE_KEY, writeAutoExpandEditActivity } from "../transcriptSettings";
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

const buttonWith = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(text));

const activityToggle = () => host.querySelector<HTMLButtonElement>("[data-activity-group] > button")!;
const openActivity = () => act(() => activityToggle().click());

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value), removeItem: (key: string) => store.delete(key) });
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
  vi.unstubAllGlobals();
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
  it("starts with the edit collapsed and reveals its first hunk on request", () => {
    mount([fileChange()]);
    expect(activityToggle().getAttribute("aria-expanded")).toBe("false");
    expect(host.textContent).not.toContain("lock_scoped");
    openActivity();
    expect(host.textContent).toContain("lock_scoped");
    expect(host.querySelector(".stx")).not.toBeNull();
  });

  it("opens short edits when enabled and preserves the reader's manual closure", async () => {
    writeAutoExpandEditActivity(true);
    mount([fileChange()]);
    expect(activityToggle().getAttribute("aria-expanded")).toBe("true");
    expect(host.textContent).toContain("lock_scoped");
    await act(async () => { activityToggle().click(); });
    mount([fileChange()]);
    expect(activityToggle().getAttribute("aria-expanded")).toBe("false");
  });

  it("updates a mounted transcript when the preference changes", async () => {
    mount([fileChange()]);
    expect(activityToggle().getAttribute("aria-expanded")).toBe("false");
    await act(async () => { writeAutoExpandEditActivity(true); window.dispatchEvent(new Event("storage")); });
    expect(activityToggle().getAttribute("aria-expanded")).toBe("true");
    await act(async () => { writeAutoExpandEditActivity(false); window.dispatchEvent(new Event("storage")); });
    expect(activityToggle().getAttribute("aria-expanded")).toBe("false");
  });

  it("keeps a long activity group collapsed even when edit expansion is enabled", () => {
    writeAutoExpandEditActivity(true);
    mount([
      event(1, "tool.completed", { title: "Read a.rs", data: { type: "readFile", path: "a.rs" } }),
      event(2, "tool.completed", { title: "Read b.rs", data: { type: "readFile", path: "b.rs" } }),
      event(3, "tool.completed", { title: "Read c.rs", data: { type: "readFile", path: "c.rs" } }),
      fileChange({ id: 4, sequence: 4, itemId: "i-4" }),
    ]);
    expect(activityToggle().getAttribute("aria-expanded")).toBe("false");
  });

  it("folds the remaining hunks behind a bar rather than truncating the patch", () => {
    mount([fileChange()]);
    openActivity();
    expect(host.textContent).not.toContain("current_generation");
    const fold = buttonWith("more hunk");
    expect(fold?.textContent).toMatch(/1 more hunk\b.*expand/);

    act(() => fold!.click());
    expect(host.textContent).toContain("current_generation");
  });

  it("keeps the diffstat and full path discoverable on the summary row", () => {
    mount([fileChange()]);
    openActivity();
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
      // A plain shell command: build, test and lint runs draw as check rows.
      title: "bun run migrate",
      data: { type: "commandExecution", command: "bun run migrate", aggregatedOutput: "92 rows\n0 skipped", ...data },
      ...overrides,
    });

  async function openGroup(events: AgentEvent[]) {
    mount(events);
    act(() => buttonWith("Ran 1 command")!.click());
  }

  it("never prints a command's exit code, and flags failed work for the agent", async () => {
    await openGroup([command({ exitCode: 2 })]);
    expect(host.textContent).not.toMatch(/exit \S/);
    expect(buttonWith("needs the agent")).toBeDefined();
    expect(buttonWith("needs attention")).toBeUndefined();
  });

  it("says nothing about a zero exit", async () => {
    await openGroup([command({ exitCode: 0 })]);
    expect(host.textContent).not.toMatch(/exit \S/);
    expect(buttonWith("needs the agent")).toBeUndefined();
  });

  it("renders no exit code when the provider reports none", async () => {
    await openGroup([command({})]);
    expect(host.textContent).not.toMatch(/exit \S/);
  });

  it("expands into a terminal block with a prompt line and dimmed output", async () => {
    await openGroup([command({ exitCode: 0 })]);
    act(() => buttonWith("Ran bun run migrate")!.click());
    expect(host.textContent).toContain("❯");
    expect(host.textContent).toContain("92 rows");
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
    openActivity();
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
    expect(host.textContent).not.toContain("Explored");
  });

  it("puts each action in the shared activity section with the patch still visible", () => {
    mount([read(1, "src-tauri/src/lib.rs"), fileChange({ id: 2, sequence: 2, itemId: "i-2" })]);
    openActivity();
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
    openActivity();
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
    // Checks are labelled by the command as typed; the order is what matters.
    const expanded = host.querySelector("[data-activity-group] > div:last-child")!.textContent ?? "";
    expect(expanded.indexOf("bun test")).toBeLessThan(expanded.indexOf("Next step"));
    expect(expanded.indexOf("Next step")).toBeLessThan(expanded.indexOf("bun run check"));
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

  it("draws a thought once when the forest catches up with it", () => {
    // A thought carries no provider item id on most harnesses, so its live row
    // and its stored twin are keyed from two id spaces that never agree. The
    // merge has to recognise them as one row on the text itself, or the Thinking
    // card prints the same paragraph twice.
    const thought = "Confirmed: while a turn is live, messages get miscategorized as tools.";
    const entries = [
      forestEntry("e1", null, 1, "user.message", { text: "fix the transcript", role: "user", status: "completed" }),
      forestEntry("e2", "e1", 41, "reasoning.completed", { text: thought, status: "completed" }),
    ];
    const live = [
      event(1, "message.completed", { itemId: null, role: "user", status: "completed", text: "fix the transcript" }),
      event(41, "reasoning.completed", { itemId: null, status: "completed", text: thought }),
    ];
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={entries} activeLeafId="e2" onResolve={() => {}} />);
    });
    // One card, settled, holding the thought once. It is the same component in
    // both states: nothing here hides thinking, it stops repeating it.
    const thinking = host.querySelector("[data-thinking]");
    expect(host.querySelectorAll("[data-thinking]")).toHaveLength(1);
    expect(thinking?.getAttribute("data-thinking")).toBe("completed");
    expect(thinking?.querySelector("summary")?.textContent).toContain("Thought for a moment");
    // Once as the collapsed summary's preview, once in the body it opens to.
    expect(thinking?.querySelector(".md")?.textContent?.split(thought).length ?? 0).toBe(2);
  });

  it("keeps one thought card on screen when the body it streamed arrives", () => {
    // A thought's live row and its stored twin are paired on text, so their ids
    // never met. Dropping the doubled row is only half of it. The survivor also
    // has to be keyed as the row already on screen, or the card the reader was
    // watching mid-thought plays its exit while the settled one enters and both
    // are drawn at once, which is the same double this branch set out to remove.
    //
    // The DOM node itself cannot survive the swap, and is not meant to: the
    // streaming state is a card and the settled state a collapsed `details`, one
    // component with two shapes. What must not happen is both at once.
    const opened = "Confirmed: while a turn is live, messages get misc";
    const whole = "Confirmed: while a turn is live, messages get miscategorized as tools.";
    const live = [
      event(1, "message.completed", { itemId: null, role: "user", status: "completed", text: "fix the transcript" }),
      event(0, "reasoning.started", { itemId: null, status: "streaming", text: opened }),
    ];
    const entries = [
      forestEntry("e1", null, 1, "user.message", { text: "fix the transcript", role: "user", status: "completed" }),
      forestEntry("e2", "e1", 41, "reasoning.completed", { text: whole, status: "completed" }),
    ];
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={[]} onResolve={() => {}} />);
    });
    const streaming = host.querySelector("[data-thinking]");
    expect(streaming?.getAttribute("data-thinking")).toBe("streaming");
    expect(streaming?.textContent).toContain(opened);
    act(() => {
      root.render(<AgentConversation session={session} events={live} forestEntries={entries} activeLeafId="e2" onResolve={() => {}} />);
    });
    const cards = [...host.querySelectorAll("[data-thinking]")];
    expect(cards).toHaveLength(1);
    expect(cards[0].getAttribute("data-thinking")).toBe("completed");
    expect(cards[0].textContent).toContain(whole);
  });
});

describe("thinking visibility", () => {
  const hideThinking = () => localStorage.setItem(SHOW_THINKING_STORAGE_KEY, "false");

  it("keeps the pulsing row and drops the text for a streaming thought", () => {
    hideThinking();
    mount([event(1, "reasoning.started", { itemId: null, status: "streaming", text: "Checking the reducer" })]);
    const row = host.querySelector("[data-thinking]")!;
    expect(row.getAttribute("data-thinking")).toBe("streaming");
    expect(row.querySelector("[data-thinking-row]")).not.toBeNull();
    expect(row.textContent).toContain("Thinking");
    expect(host.textContent).not.toContain("Checking the reducer");
  });

  it("leaves no row, text or gap behind a settled thought", () => {
    hideThinking();
    mount([event(1, "reasoning.completed", { itemId: null, status: "completed", text: "All done thinking" })]);
    expect(host.querySelector("[data-thinking]")).toBeNull();
    expect(host.querySelector("[data-thinking-row]")).toBeNull();
    expect(host.querySelector("[data-conversation-content]")?.children).toHaveLength(0);
    expect(host.textContent).not.toContain("All done thinking");
    expect(host.textContent).not.toContain("Thought for");
  });

  it("still shows thinking by default", () => {
    mount([event(1, "reasoning.completed", { itemId: null, status: "completed", text: "All done thinking" })]);
    expect(host.querySelector('[data-thinking="completed"]')).not.toBeNull();
    expect(host.textContent).toContain("All done thinking");
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

describe("activity timeline", () => {
  it("draws one borderless Worked for header over the run", () => {
    mount([event(1, "command.completed", {
      title: "git status",
      createdAt: "2026-01-01T00:00:00.000Z",
      data: { type: "commandExecution", command: "git status", durationMs: 4000, exitCode: 0 },
    })]);
    const group = host.querySelector("[data-activity-group]")!;
    expect(group.className).not.toMatch(/\bborder\b/);
    expect(buttonWith("Worked for 4s")).toBeDefined();
    expect(host.textContent).not.toContain("Activity");
  });

  it("wears the git mark on git work and a connector logo on a known MCP server", () => {
    mount([
      event(1, "command.completed", { itemId: "g", title: "git diff", data: { type: "commandExecution", command: "git diff", exitCode: 0 } }),
      event(2, "tool.completed", { itemId: "n", title: "search", data: { name: "mcp__notion__search" } }),
      event(3, "tool.completed", { itemId: "x", title: "lookup", data: { name: "mcp__acme__lookup" } }),
    ]);
    act(() => host.querySelector<HTMLButtonElement>("[data-activity-group] > button")!.click());
    const rows = [...host.querySelectorAll("[data-tool-row]")];
    expect(rows).toHaveLength(3);
    // Stroke-drawn git mark, the Notion path, and the lucide wrench fallback.
    expect(rows[0].querySelector('svg[stroke="currentColor"]')).not.toBeNull();
    expect(rows[1].textContent).toContain("Used notion");
    expect(rows[1].querySelector("svg path[fill='currentColor']")).not.toBeNull();
    expect(rows[2].querySelector(".lucide-wrench")).not.toBeNull();
  });
});

describe("mid-turn narration", () => {
  // The live channel carries persisted frames under their forest kind. Before
  // the forest poll catches up, the model's updates must still split the run
  // and read as prose, not fold into the Working group as "used N tools".
  it("shows each update between the runs it narrates, before the forest has it", () => {
    const command = (id: number, cmd: string) => event(id, "command.completed", { itemId: `c${id}`, title: cmd, data: { type: "commandExecution", command: cmd, exitCode: 0 } });
    mount([
      event(1, "user.message", { itemId: "u1", role: "user", text: "Do it yourself" }),
      event(2, "message.started", { itemId: "m1", role: "assistant", status: "started", text: "" }),
      event(3, "assistant.message", { itemId: "m1", role: "assistant", text: "Now committing the test contract." }),
      command(4, "git add testing"),
      command(5, "git commit"),
      event(6, "message.started", { itemId: "m2", role: "assistant", status: "started", text: "" }),
      event(7, "assistant.message", { itemId: "m2", role: "assistant", text: "Next I'm running the checks." }),
      command(8, "bun run test"),
    ]);
    const text = host.textContent ?? "";
    expect(text).toContain("Now committing the test contract.");
    expect(text).toContain("Next I'm running the checks.");
    expect(text).not.toMatch(/used \d+ tools?/i);
    expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(2);
    expect(text.indexOf("Now committing")).toBeLessThan(text.indexOf("Next I'm running"));
    // One bubble per message: the replay and its forest twin never double up.
    expect(text.split("Now committing the test contract.")).toHaveLength(2);
  });
});

describe("check rows", () => {
  const run = (id: number, command: string, data: Record<string, unknown>, overrides: Partial<AgentEvent> = {}) =>
    event(id, "command.completed", { itemId: `c${id}`, title: command, data: { type: "commandExecution", command, ...data }, ...overrides });

  it("lists a run's checks under the folded header, like the Verifying card", () => {
    mount([
      run(1, "rg TokenStore src", { exitCode: 0 }),
      run(2, "bun run test", { exitCode: 0, durationMs: 41000, aggregatedOutput: "Tests  2677 passed (2677)" }),
      run(3, "bun run build", { exitCode: 0, aggregatedOutput: "✓ built in 6.76s" }),
    ]);
    const list = host.querySelector("[data-check-list]")!;
    expect(list).not.toBeNull();
    const rows = [...list.querySelectorAll("[data-tool-row]")];
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("bun run test");
    expect(rows[0].textContent).toContain("Test");
    expect(rows[0].textContent).toContain("2677 tests passed");
    expect(rows[0].textContent).toContain("Passed");
    expect(rows[0].querySelector(".text-success")).not.toBeNull();
    expect(rows[1].textContent).toContain("built in 6.76s");
    // The exploratory search is work, not a result: it stays folded away.
    expect(list.textContent).not.toContain("rg TokenStore");
  });

  it("marks a check failed on a nonzero exit or reported failures, and running while live", () => {
    mount([
      run(1, "cargo test", { exitCode: 0, aggregatedOutput: "test result: FAILED. 214 passed; 2 failed; 0 ignored" }),
      run(2, "bun run build", {}, { kind: asWireKind("command.started"), status: "inProgress" }),
    ]);
    const rows = [...host.querySelectorAll("[data-check-list] [data-tool-row]")];
    expect(rows[0].textContent).toContain("Failed");
    expect(rows[0].textContent).toContain("2 failed · 214 passed");
    expect(rows[1].textContent).toContain("Running");
    expect(rows[1].querySelector(".animate-spin.text-warning")).not.toBeNull();
  });

  it("fails a zero-exit run whose test file failed to collect, and flags the group for the agent", () => {
    // `vitest run | cat` without pipefail exits 0 while a suite failed.
    mount([run(1, "bunx vitest run | cat", { exitCode: 0, aggregatedOutput: " Test Files  1 failed | 1 passed (2)\n      Tests  1 passed (1)" })]);
    const row = host.querySelector("[data-check-list] [data-tool-row]")!;
    expect(row.textContent).toContain("Failed");
    expect(row.textContent).toContain("1 file failed · 1 passed");
    expect(row.textContent).not.toContain("Passed");
    expect(buttonWith("needs the agent")).toBeDefined();
    expect(buttonWith("needs attention")).toBeUndefined();
  });

  it("keeps only the latest run of a repeated check", () => {
    mount([
      run(1, "bun run test", { exitCode: 1, aggregatedOutput: "Tests  1 failed | 10 passed (11)" }),
      run(2, "bun run test", { exitCode: 0, aggregatedOutput: "Tests  11 passed (11)" }),
    ]);
    const rows = [...host.querySelectorAll("[data-check-list] [data-tool-row]")];
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain("11 tests passed");
  });
});

describe("failed run expansion", () => {
  const run = (id: number, command: string, data: Record<string, unknown>) =>
    event(id, "command.completed", { itemId: `c${id}`, title: command, data: { type: "commandExecution", command, ...data } });

  it("opens a failed run on its failures, and keeps the rest one click away", () => {
    mount([
      run(1, "bun run test", { exitCode: 1, aggregatedOutput: "1 failed" }),
      run(2, "git status", { exitCode: 0 }),
      run(3, "cargo build", { exitCode: 0 }),
    ]);
    expect(buttonWith("needs the agent")).toBeDefined();
    act(() => buttonWith("needs the agent")!.click());
    const rows = [...host.querySelectorAll("[data-tool-row]")];
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain("bun run test");
    expect(rows[0].textContent).not.toContain("git status");
    act(() => host.querySelector<HTMLButtonElement>("[data-show-all-steps]")!.click());
    expect([...host.querySelectorAll("[data-tool-row]")]).toHaveLength(3);
    expect(host.querySelector("[data-show-all-steps]")).toBeNull();
  });

  it("still opens a healthy run whole", () => {
    act(() => {
      root.render(<AgentConversation session={{ ...session, status: "idle" }} events={[run(1, "bun run migrate", { exitCode: 0 }), run(2, "bun run seed", { exitCode: 0 })]} onResolve={() => {}} />);
    });
    act(() => buttonWith("Ran 2 commands")!.click());
    expect([...host.querySelectorAll("[data-tool-row]")]).toHaveLength(2);
    expect(host.querySelector("[data-show-all-steps]")).toBeNull();
  });
});

describe("turn liveness", () => {
  const started = () => event(1, "command.started", {
    itemId: "c1", title: "bun run test", status: "inProgress",
    data: { type: "commandExecution", command: "bun run test" },
  });

  it("keeps a live check running while the turn is active", () => {
    mount([started()]);
    expect(buttonWith("Working")).toBeDefined();
    expect(host.textContent).toContain("Running");
  });

  it("stops claiming live work once the turn is over", () => {
    act(() => {
      root.render(<AgentConversation session={{ ...session, status: "idle" }} events={[started()]} onResolve={() => {}} />);
    });
    expect(buttonWith("Working")).toBeUndefined();
    expect(host.textContent).not.toContain("Running");
    // The item keeps the status the provider gave it; the presentation just
    // stops claiming a turn that is no longer running.
    expect(host.textContent).toContain("Pending");
    expect(buttonWith("Ran 1 command")).toBeDefined();
  });
});

describe("forest bookkeeping", () => {
  const renderForest = (entries: SessionEntry[]) => act(() => {
    root.render(<AgentConversation session={session} events={[]} forestEntries={entries} activeLeafId={entries.at(-1)?.id} onResolve={() => {}} />);
  });

  it("draws checkpoints and branch summaries as faint lines, not cards", () => {
    renderForest([
      forestEntry("u", null, 1, "user.message", { text: "Go", itemId: "u" }),
      forestEntry("c", "u", 2, "checkpoint", { summary: "Workers own isolated paths" }),
      forestEntry("b", "c", 3, "branch.summary", { summary: "Explored the alternate\nKept the store" }),
    ]);
    const lines = [...host.querySelectorAll("[data-forest-line]")];
    expect(lines).toHaveLength(2);
    expect(host.textContent).toContain("Workers own isolated paths");
    for (const line of lines) expect(line.className).not.toContain("rounded-xl");
    expect(host.textContent).toContain("Explored the alternate");
  });

  it("opens a long single-paragraph summary, and shows the whole text, first paragraph included", () => {
    const long = "Workers own isolated paths and every write is scoped to the worktree the policy engine granted, so a stray edit can never land in the parent checkout.";
    renderForest([
      forestEntry("u", null, 1, "user.message", { text: "Go", itemId: "u" }),
      forestEntry("c", "u", 2, "checkpoint", { summary: long }),
      forestEntry("b", "c", 3, "branch.summary", { summary: `${long}\nKept the store` }),
    ]);
    const details = [...host.querySelectorAll<HTMLDetailsElement>("details[data-forest-line]")];
    expect(details).toHaveLength(2);
    const bodies = details.map(detail => detail.querySelector("[data-forest-detail]")!.textContent);
    expect(bodies[0]).toBe(long);
    expect(bodies[1]).toBe(`${long}\nKept the store`);
    // The body is not a truncating element.
    for (const detail of details) expect(detail.querySelector("[data-forest-detail]")!.className).not.toContain("truncate");
  });

  it("shows no card for a session-start branch summary", () => {
    renderForest([
      forestEntry("root", null, 1, "branch.summary", { summary: "Session started" }),
      forestEntry("u", "root", 2, "user.message", { text: "Hello there", itemId: "u" }),
    ]);
    expect(host.textContent).toContain("Hello there");
    expect(host.textContent).not.toContain("Session started");
    expect(host.textContent).not.toContain("Branch summary");
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
