// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { TasksPane } from "./TasksPane";

// Contract: testing/feat-dock-tasks.md §1–§3.

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, workspaceId: "w", harness: "codex", label: `Worker ${id}`, status: "working", startedAt: "2026-08-25T10:00:00Z",
  endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "m", restorationMode: "hot",
  continuationFidelity: "native", kind: "worker", parentSessionId: "parent", ...overrides,
});

const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId, parentSessionId: "parent", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "k",
  resultStatus: "pending", retryCount: 0, lastResult: null, lastActivityAt: "2026-08-25T10:00:00Z", ...overrides,
} as WorkerRuntimeRecord);

const queued: QueuedWorkerRequest = {
  id: "q1", parentSessionId: "parent", workspaceId: "w", turnId: "t", sequence: 1, attemptCount: 0,
  request: { role: "implementation", objective: "Update the auth serializer", reason: "owned_path_conflict" },
  actualModel: "gpt-terra", queueStatus: "queued", dispatchedSessionId: null, lastError: null,
  blockedAt: null, claimedAt: null, expiresAt: "later", createdAt: "now", updatedAt: "now",
} as QueuedWorkerRequest;

let container: HTMLDivElement;
let root: Root;

async function mount(props: Partial<Parameters<typeof TasksPane>[0]> = {}) {
  await act(async () => {
    root.render(<TasksPane
      sessions={props.sessions ?? []}
      acknowledged={props.acknowledged ?? new Set()}
      onAcknowledge={props.onAcknowledge ?? (() => undefined)}
      {...props}
    />);
  });
}

const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

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

describe("TasksPane roster", () => {
  it("speaks the worker status vocabulary with detail", async () => {
    await mount({
      sessions: [session("a"), session("b", { status: "ready" })],
      runtimes: [
        runtime("a"),
        runtime("b", { lifecycleState: "completed", resultStatus: "reported", lastResult: { status: "completed", summary: "All 42 auth tests pass" } }),
      ],
    });
    expect(container.textContent).toContain("WORKING");
    expect(container.textContent).toContain("implementation");
    expect(container.textContent).toContain("DONE");
    expect(container.textContent).toContain("All 42 auth tests pass");
  });

  it("keeps retries attributed", async () => {
    await mount({ sessions: [session("a")], runtimes: [runtime("a", { retryCount: 3 })] });
    expect(container.textContent).toContain("3 retries");
  });

  it("states why a delegation waits", async () => {
    await mount({ queue: [queued] });
    expect(container.textContent).toContain("Update the auth serializer");
    expect(container.textContent).toContain("QUEUED");
    expect(container.textContent).toContain("owned_path_conflict");
  });

  it("renders shells as one row", async () => {
    await mount({ terminalActivity: { running: 2, attention: false } });
    expect(container.textContent).toContain("2 shells running");
  });

  it("says when nothing is in flight", async () => {
    await mount({});
    expect(container.textContent).toContain("Nothing is running in the background for this session.");
  });
});

describe("TasksPane failures", () => {
  const failed = () => ({
    sessions: [session("f", { status: "failed" })],
    runtimes: [runtime("f", { lifecycleState: "failed" })],
  });

  it("reads differently and stays until dismissed", async () => {
    await mount(failed());
    expect(container.textContent).toContain("FAILED");
    expect(container.querySelector('button[aria-label="Dismiss failure of Worker f"]')).not.toBeNull();
    await mount({ sessions: [session("a")], runtimes: [runtime("a")] });
    expect(container.querySelector('button[aria-label^="Dismiss failure"]')).toBeNull();
  });

  it("hands dismissal to the host and honours the acknowledged set", async () => {
    const onAcknowledge = vi.fn();
    await mount({ ...failed(), onAcknowledge });
    await click(container.querySelector('button[aria-label="Dismiss failure of Worker f"]')!);
    expect(onAcknowledge).toHaveBeenCalledWith("f");

    await mount({ ...failed(), acknowledged: new Set(["f"]) });
    expect(container.textContent).not.toContain("FAILED");
    expect(container.textContent).toContain("Nothing is running in the background for this session.");
  });
});

describe("TasksPane actions", () => {
  it("opens and expands through the existing wiring", async () => {
    const onOpenSession = vi.fn();
    const onExpandWorker = vi.fn();
    await mount({ sessions: [session("a")], runtimes: [runtime("a")], onOpenSession, onExpandWorker });
    await click(container.querySelector('button[aria-label="Open worker Worker a"]')!);
    expect(onOpenSession).toHaveBeenCalledWith("a");
    await click(container.querySelector('button[aria-label="Expand worker Worker a"]')!);
    expect(onExpandWorker).toHaveBeenCalledWith("a");
  });

  it("offers retry on failed rows only", async () => {
    const onRetryWorker = vi.fn();
    await mount({
      sessions: [session("a"), session("f", { status: "failed" })],
      runtimes: [runtime("a"), runtime("f", { lifecycleState: "failed" })],
      onRetryWorker,
    });
    expect(container.querySelector('button[aria-label="Retry worker Worker a"]')).toBeNull();
    await click(container.querySelector('button[aria-label="Retry worker Worker f"]')!);
    expect(onRetryWorker).toHaveBeenCalledWith("f");
  });

  it("routes the shells row to the terminal pane", async () => {
    const onOpenTerminal = vi.fn();
    await mount({ terminalActivity: { running: 1, attention: false }, onOpenTerminal });
    await click(container.querySelector('button[aria-label="Reveal shells in the terminal pane"]')!);
    expect(onOpenTerminal).toHaveBeenCalledTimes(1);
  });
});
