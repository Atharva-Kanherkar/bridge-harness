// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import type { AgentAsk, AgentNode, AgentRun, AgentsScope } from "./agentsModel";
import { AgentsPane, PinnedAgentsTray } from "./AgentsPane";

// Contract: testing/feat-agents-pane.md §3.

const NOW = Date.parse("2026-08-25T10:05:00Z");

function node(overrides: Partial<AgentNode> = {}): AgentNode {
  return {
    id: "w1", source: "worker", harness: "claude", name: "Implementation · strong", depth: 0,
    status: { tone: "working", label: "WORKING" }, steps: [], children: [], sessionId: "w1",
    scope: [], counters: { files: 0, additions: 0, deletions: 0, toolCalls: 0 }, ...overrides,
  } as AgentNode;
}

function run(agents: AgentNode[], overrides: Partial<AgentRun> = {}): AgentRun {
  return {
    rootSessionId: "root", title: "Add refresh-token rotation", harness: "claude",
    startedAt: "2026-08-25T10:00:00Z", agents,
    census: { running: agents.length, needsYou: 0, done: 0, failed: 0, queued: 0, workers: agents.length, subagents: 0, costUsd: 0 },
    ...overrides,
  };
}

const ASK: AgentAsk = { eventId: 42, sessionId: "root", title: "Verification · strong wants to run", command: "bun test src/auth", cwd: ".worktrees/verify-7c1", ownedPaths: ["src/auth/**"], detail: "Run a command in its worktree?" };

let container: HTMLDivElement;
let root: Root;

async function mount(props: Partial<Parameters<typeof AgentsPane>[0]> = {}) {
  await act(async () => {
    root.render(<AgentsPane
      runs={[]}
      expanded={new Set()}
      pinned={new Set()}
      scope={"this-chat" as AgentsScope}
      onScope={() => undefined}
      onToggleExpanded={() => undefined}
      onTogglePinned={() => undefined}
      now={NOW}
      {...props}
    />);
  });
}

const click = async (element: Element | null | undefined) => {
  expect(element, "expected the element to be in the tree").toBeTruthy();
  await act(async () => { element!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
};

const text = () => container.textContent ?? "";
const row = (id: string) => container.querySelector(`[data-agent-row="${id}"]`);
const byLabel = (name: string) => Array.from(container.querySelectorAll("button")).find(button => button.getAttribute("aria-label") === name);

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("AgentsPane rows", () => {
  it("collapses to one live line and opens in place", async () => {
    const onToggleExpanded = vi.fn();
    const expanded = node({
      steps: [
        { id: "s1", verb: "Read", target: "src/auth/store.rs", state: "done" },
        { id: "s2", verb: "Edit", target: "src/auth/store.rs", extra: "+18 −6", additions: 18, deletions: 6, state: "live" },
      ],
      liveLine: { id: "s2", verb: "Edit", target: "src/auth/store.rs", extra: "+18 −6", state: "live" },
      startedAt: "2026-08-25T10:00:00Z",
      model: "opus-5.5", effort: "high", branch: "feat/token-rotation", scope: ["src/auth/**"],
      objective: "Add refresh-token rotation to the SQLite token store.",
      counters: { files: 2, additions: 22, deletions: 7, toolCalls: 18, contextPercent: 34, costUsd: 0.41 },
    });
    await mount({ runs: [run([expanded])], onToggleExpanded });
    expect(text()).toContain("Implementation · strong");
    expect(text()).toContain("worker");
    expect(text()).toContain("Edit");
    expect(text()).toContain("+18");
    expect(text()).not.toContain("feat/token-rotation");
    expect(text()).not.toContain("Transcript");

    await click(byLabel("Expand Implementation · strong"));
    expect(onToggleExpanded).toHaveBeenCalledWith("w1");

    const actions = { onDrillIn: () => undefined, onSteer: async () => undefined, onStopWorker: () => undefined };
    await mount({ runs: [run([expanded])], onToggleExpanded, expanded: new Set(["w1"]), ...actions });
    expect(text()).toContain("Add refresh-token rotation to the SQLite token store.");
    expect(text()).toContain("opus-5.5 · high");
    expect(text()).toContain("feat/token-rotation");
    expect(text()).toContain("src/auth/**");
    expect(text()).toContain("2 files");
    expect(text()).toContain("18 tool calls");
    expect(text()).toContain("ctx 34%");
    expect(text()).toContain("$0.41");
    for (const action of ["Transcript", "Steer", "Stop", "Pin"]) expect(text()).toContain(action);
  });

  it("labels Pin for this row only, not for whether anything is pinned", async () => {
    await mount({ runs: [run([node(), node({ id: "w2", sessionId: "w2", name: "Docs · fast" })])], expanded: new Set(["w1", "w2"]), pinned: new Set(["w2"]) });
    const pins = Array.from(container.querySelectorAll("button[aria-pressed]")).filter(button => /Pin/.test(button.textContent ?? ""));
    expect(pins.map(button => [button.textContent, button.getAttribute("aria-pressed")])).toEqual([["Pin", "false"], ["Pinned", "true"]]);
  });

  it("nests a child with an indent and a hairline, never a box", async () => {
    const child = node({ id: "w1:sub:toolu_1", source: "subagent", name: "Explore", parentId: "w1", depth: 1, status: { tone: "done", label: "DONE" } });
    await mount({ runs: [run([node({ children: [child] })])], expanded: new Set(["w1"]) });
    const nested = row("w1:sub:toolu_1");
    expect(nested?.className).toContain("ml-[15px]");
    expect(nested?.className).toContain("border-l");
    expect(nested?.className).toContain("pl-2");
    expect(text()).toContain("subagent");
  });

  it("answers an ask in the row through the injected resolver", async () => {
    const onResolveAsk = vi.fn(async () => undefined);
    const onOpenSession = vi.fn();
    const asking = node({ id: "w2", name: "Verification · strong", ask: ASK, status: { tone: "waiting", label: "NEEDS YOU" } });
    await mount({ runs: [run([asking])], onResolveAsk, onOpenSession });
    expect(text()).toContain("NEEDS YOU");
    expect(text()).toContain("bun test src/auth");
    expect(text()).toContain(".worktrees/verify-7c1");
    const approve = Array.from(container.querySelectorAll("button")).find(button => button.textContent === "Approve");
    await click(approve);
    expect(onResolveAsk).toHaveBeenCalledWith(ASK, "accept");
    expect(onOpenSession).not.toHaveBeenCalled();
  });

  it("keeps a refused ask in NEEDS YOU and shows why", async () => {
    const onResolveAsk = vi.fn(async () => { throw new Error("the worker went away"); });
    const asking = node({ id: "w2", name: "Verification", ask: ASK, status: { tone: "waiting", label: "NEEDS YOU" } });
    await mount({ runs: [run([asking])], onResolveAsk });
    await click(Array.from(container.querySelectorAll("button")).find(button => button.textContent === "Deny"));
    expect(text()).toContain("NEEDS YOU");
    expect(container.querySelector("[role=alert]")?.textContent).toBe("the worker went away");
  });

  it("shows a failure as a stable code and a legible last step", async () => {
    const onRetryWorker = vi.fn();
    const failed = node({
      status: { tone: "failed", label: "FAILED" },
      failureCode: "sandbox_denied",
      liveLine: { id: "s", verb: "Edit", target: "ci.yml", state: "failed" },
    });
    await mount({ runs: [run([failed])], onRetryWorker, onDrillIn: () => undefined });
    expect(text()).toContain("sandbox_denied");
    expect(text()).toContain("Last step: Edit ci.yml");
    expect(text()).not.toContain("ENOENT");
    await click(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.startsWith("Retry")));
    expect(onRetryWorker).toHaveBeenCalledWith("w1");
  });

  it("keeps a finished row on screen, dimmed", async () => {
    const done = node({ id: "w3", name: "Docs · fast", status: { tone: "done", label: "DONE" }, startedAt: "2026-08-25T10:00:00Z", endedAt: "2026-08-25T10:01:48Z" });
    await mount({ runs: [run([done], { census: { running: 0, needsYou: 0, done: 1, failed: 0, queued: 0, workers: 1, subagents: 0, costUsd: 0 } })] });
    expect(row("w3")).toBeTruthy();
    expect(text()).toContain("1m 48s");
  });

  it("drains the queue and the shell row below the agents", async () => {
    const queued = { id: "q1", actualModel: "gpt-5.6-terra", queueStatus: "queued", request: { objective: "Update the auth serializer", reason: "waits for src/auth/**" } } as unknown as QueuedWorkerRequest;
    await mount({ runs: [run([node()])], queue: [queued], terminalActivity: { running: 2, attention: false } as never });
    expect(text()).toContain("Update the auth serializer");
    expect(text()).toContain("2 shells running");
    // Queued delegations and running shells stay at the bottom, below the rows.
    const scroller = container.querySelector(".overflow-y-auto")!;
    const kids = Array.from(scroller.children);
    expect(kids[kids.length - 1].textContent).toContain("2 shells running");
    expect(kids[kids.length - 2].textContent).toContain("Update the auth serializer");
    expect(Array.from(scroller.children).findIndex(child => child.textContent?.includes("Implementation · strong")))
      .toBeLessThan(kids.length - 2);
  });

  it("steers from the row through the same call the worker's own view uses", async () => {
    const onSteer = vi.fn(async () => undefined);
    await mount({ runs: [run([node()])], expanded: new Set(["w1"]), onSteer });
    await click(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.trim() === "Steer"));
    const box = container.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(box.getAttribute("aria-label")).toBe("Steer Implementation · strong");
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "keep the old rows readable");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.trim() === "Send"));
    expect(onSteer).toHaveBeenCalledWith("w1", "keep the old rows readable");
  });

  it("drills in without the chat overlay", async () => {
    const onDrillIn = vi.fn();
    const target = node({ steps: [{ id: "s1", verb: "Read", target: "src/auth/store.rs", state: "done" }], counters: { files: 2, additions: 22, deletions: 7, toolCalls: 18, contextPercent: 34, costUsd: 0.41 } });
    await mount({ runs: [run([target])], expanded: new Set(["w1"]), onDrillIn });
    await click(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.includes("Transcript")));
    expect(onDrillIn).toHaveBeenCalledWith("w1");

    await mount({ runs: [run([target])], expanded: new Set(["w1"]), onDrillIn, drillId: "w1", onSteer: async () => undefined, onStopWorker: () => undefined });
    expect(text()).toContain("18");
    expect(text()).toContain("tool calls");
    expect(text()).toContain("context");
    expect(text()).toContain("so far");
    for (const tab of ["Activity", "Files", "Prompt", "Result"]) expect(text()).toContain(tab);
    expect(container.querySelector("textarea")?.getAttribute("aria-label")).toBe("Steer this agent. It reads it at its next step.");
    expect(Array.from(container.querySelectorAll("button")).some(button => button.textContent?.includes("Stop"))).toBe(true);
    // The overlay that used to cover the whole chat section is gone.
    expect(container.querySelector(".absolute.inset-0")).toBeNull();
    expect(container.querySelector("[role=dialog]")).toBeNull();
  });
});

describe("PinnedAgentsTray", () => {
  const pinnedRun = run([
    node({ id: "w1", name: "Implementation · strong", liveLine: { id: "s", verb: "Run", target: "cargo test auth::", state: "live" } }),
    node({ id: "w2", name: "Verification · strong", status: { tone: "waiting", label: "NEEDS YOU" }, ask: { ...ASK, command: "bun test src/auth" } }),
  ]);

  it("renders on another pane, with the chat title and a live step", async () => {
    await act(async () => {
      root.render(<PinnedAgentsTray runs={[pinnedRun]} pinned={new Set(["w1"])} now={NOW} pane="github" onOpenAgents={() => undefined} chatTitle={() => "Add refresh-token rotation"} />);
    });
    expect(text()).toContain("Pinned agents");
    expect(text()).toContain("1");
    expect(text()).toContain("Add refresh-token rotation");
    expect(text()).toContain("Run cargo test auth::");
  });

  it("keeps the ask's primary action inline", async () => {
    const onResolveAsk = vi.fn(async () => undefined);
    await act(async () => {
      root.render(<PinnedAgentsTray runs={[pinnedRun]} pinned={new Set(["w1", "w2"])} now={NOW} pane="github" onOpenAgents={() => undefined} onResolveAsk={onResolveAsk} chatTitle={() => "Add refresh-token rotation"} />);
    });
    expect(text()).toContain("NEEDS YOU");
    await click(Array.from(container.querySelectorAll("button")).filter(button => button.textContent === "Approve")[0]);
    expect(onResolveAsk).toHaveBeenCalledWith(expect.objectContaining({ command: "bun test src/auth" }), "accept");
  });

  it("collapses, and stands down on the Agents pane itself", async () => {
    await act(async () => {
      root.render(<PinnedAgentsTray runs={[pinnedRun]} pinned={new Set(["w1"])} now={NOW} pane="github" onOpenAgents={() => undefined} chatTitle={() => "Add refresh-token rotation"} />);
    });
    await click(container.querySelector("[data-pinned-tray] button"));
    expect(text()).not.toContain("Run cargo test auth::");

    await act(async () => {
      root.render(<PinnedAgentsTray runs={[pinnedRun]} pinned={new Set(["w1"])} now={NOW} pane="tasks" onOpenAgents={() => undefined} chatTitle={() => "Add refresh-token rotation"} />);
    });
    expect(container.querySelector("[data-pinned-tray]")).toBeNull();
  });

  it("renders nothing when nothing is pinned", async () => {
    await act(async () => {
      root.render(<PinnedAgentsTray runs={[pinnedRun]} pinned={new Set()} now={NOW} pane="github" onOpenAgents={() => undefined} />);
    });
    expect(container.querySelector("[data-pinned-tray]")).toBeNull();
  });
});
