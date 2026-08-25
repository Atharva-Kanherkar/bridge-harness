// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { AgentEvent, Session } from "../types";

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
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind, itemId: `i-${id}`,
  role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {},
  createdAt: "now", ...overrides,
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

  it("keeps the diffstat and path on the summary row", () => {
    mount([fileChange()]);
    expect(host.textContent).toContain("+24");
    expect(host.textContent).toContain("−3");
    expect(host.textContent).toContain("src-tauri/src/lib.rs");
  });

  it("still opens a group whose edit carries no diff only on request", () => {
    mount([event(1, "file_change.completed", { title: "lib.rs", data: { path: "src-tauri/src/lib.rs", additions: 1, deletions: 0 } })]);
    // Nothing to show inline, so the group stays folded the way it always did.
    expect(host.textContent).not.toContain("src-tauri/src/lib.rs");
    expect(buttonWith("Edited files")).toBeDefined();
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
    act(() => buttonWith("Ran commands")!.click());
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

  it("groups consecutive reads and searches under a quiet label", () => {
    mount([
      read(1, "src-tauri/src/lib.rs"),
      event(2, "tool.completed", { title: "grep", data: { name: "Grep", input: { pattern: "resume" } } }),
      fileChange({ id: 3, sequence: 3, itemId: "i-3" }),
    ]);
    // The group is already open, because the edit carries a diff.
    expect(host.textContent).toContain("Explored");
  });

  it("gives an edit a bordered card and a read a flat row", () => {
    mount([read(1, "src-tauri/src/lib.rs"), fileChange({ id: 2, sequence: 2, itemId: "i-2" })]);
    const rowFor = (label: string) => buttonWith(label)?.parentElement;
    expect(rowFor("Edited lib.rs")?.className).toContain("border-border");
    expect(rowFor("Read lib.rs")?.className).not.toContain("border-border");
  });

  it("does not reorder the transcript to tidy it", () => {
    mount([
      read(1, "a.rs"),
      fileChange({ id: 2, sequence: 2, itemId: "i-2" }),
      read(3, "b.rs"),
    ]);
    const text = host.textContent ?? "";
    // Two separate "Explored" runs, because the edit sits between them.
    expect(text.split("Explored")).toHaveLength(3);
  });
});
