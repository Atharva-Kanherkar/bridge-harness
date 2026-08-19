// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkBoard, WorkFact, WorkFactAction } from "../protocol/generated/protocol";
import { WorkView, type WorkActionOutcome } from "./WorkView";

const NOW = new Date("2026-08-19T12:00:00.000Z");

function fact(overrides: Partial<WorkFact> = {}): WorkFact {
  return {
    kind: "failed_completion_check",
    dedupeKey: "check:a-1:cargo-test",
    severity: "blocking",
    title: "cargo-test failed on Kyoto",
    detail: "A required check failed.",
    target: { kind: "completionAttempt", sessionId: "s-1", attemptId: "a-1" },
    actionableAt: "2026-08-19T11:00:00.000Z",
    observedAt: "2026-08-19T11:59:50.000Z",
    freshness: "live",
    action: { kind: "reviewCompletionCheck", sessionId: "s-1", attemptId: "a-1", checkId: "cargo-test" },
    ...overrides,
  };
}

function board(facts: WorkFact[], overrides: Partial<WorkBoard> = {}): WorkBoard {
  return {
    facts,
    tasks: [],
    latestRun: null,
    settings: {
      briefing: null,
      enabledConnectorInstances: [],
      refreshOnFocus: false,
      refreshIntervalMinutes: null,
      cooldownMinutes: 15,
      limits: { maxWallSeconds: 600, maxTurns: 12, maxToolCalls: 24, maxOutputTokens: null, costCeilingMicrousd: null },
    },
    suggestions: { state: "unavailable", detail: null },
    ...overrides,
  };
}

let host: HTMLDivElement;
let root: Root;

const ok = async (): Promise<WorkActionOutcome> => ({ ok: true });

function render(props: Partial<Parameters<typeof WorkView>[0]> = {}) {
  act(() => {
    root.render(
      <WorkView board={board([fact()])} onRefresh={() => {}} onAction={ok} now={NOW} {...props} />,
    );
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const text = () => host.textContent ?? "";
const buttons = () => Array.from(host.querySelectorAll("button"));
const buttonNamed = (label: string) => buttons().find(button => button.textContent?.includes(label));

describe("facts", () => {
  it("renders a row and an action for every fact kind", () => {
    render({
      board: board([
        fact({ dedupeKey: "1", kind: "failed_completion_check", action: { kind: "reviewCompletionCheck", sessionId: "s", attemptId: "a", checkId: "c" } }),
        fact({ dedupeKey: "2", kind: "actionable_approval", title: "Approve command", target: { kind: "session", sessionId: "s" }, action: { kind: "answerApproval", sessionId: "s", approvalSequence: 4 } }),
        fact({ dedupeKey: "3", kind: "blocked_worker_queue_item", title: "3 queued workers are parked", target: { kind: "workerQueueItem", queueId: "q-1", workspaceId: "w-1" }, action: { kind: "answerApproval", sessionId: "s", approvalSequence: null } }),
        fact({ dedupeKey: "4", kind: "workspace_behind_base", severity: "attention", title: "Kyoto has drifted behind its base branch", target: { kind: "workspace", workspaceId: "w-1", sessionId: "s" }, action: { kind: "refreshWorkspaceBase", sessionId: "s", workspaceId: "w-1" } }),
      ]),
    });
    expect(host.querySelectorAll("li")).toHaveLength(4);
    expect(buttonNamed("Review check")).toBeTruthy();
    expect(buttonNamed("Answer")).toBeTruthy();
    expect(buttonNamed("Fast-forward")).toBeTruthy();
  });

  it("renders facts in the order given, and bands by severity without reordering", () => {
    render({
      board: board([
        fact({ dedupeKey: "b1", severity: "blocking", title: "first blocking" }),
        fact({ dedupeKey: "b2", severity: "blocking", title: "second blocking" }),
        fact({ dedupeKey: "a1", severity: "attention", title: "an attention" }),
      ]),
    });
    const titles = Array.from(host.querySelectorAll("li")).map(row => row.textContent ?? "");
    expect(titles[0]).toContain("first blocking");
    expect(titles[1]).toContain("second blocking");
    expect(titles[2]).toContain("an attention");
    expect(text()).toContain("Blocking");
    expect(text()).toContain("Attention");
  });

  it("leaves a band with no facts out entirely", () => {
    render({ board: board([fact({ severity: "attention" })]) });
    expect(text()).toContain("Attention");
    expect(text()).not.toContain("Blocking");
    expect(text()).not.toContain("Info");
  });

  it("names severity, source and freshness in text for a screen reader", () => {
    render();
    // The visible title is aria-hidden and the announcement carries the claim, so
    // the row reads identically with no colour available.
    expect(text()).toContain("Blocking, Completion check: cargo-test failed on Kyoto. seen just now.");
  });

  it("gives the list an accessible name while loading", () => {
    render({ board: undefined });
    expect(host.querySelector("[aria-label='Loading work']")).toBeTruthy();
  });
});

describe("freshness", () => {
  const divergence = (freshness: WorkFact["freshness"], action: WorkFactAction) =>
    board([
      fact({
        kind: "workspace_behind_base",
        severity: "attention",
        title: "Kyoto has drifted behind its base branch",
        detail: "41 behind, 2 ahead of origin/main.",
        target: { kind: "workspace", workspaceId: "w-1", sessionId: "s" },
        observedAt: "2026-08-19T11:36:00.000Z",
        freshness,
        action,
      }),
    ]);

  it("offers a fast-forward for a live reading", () => {
    render({ board: divergence("live", { kind: "refreshWorkspaceBase", sessionId: "s", workspaceId: "w-1" }) });
    expect(buttonNamed("Fast-forward")).toBeTruthy();
    expect(buttonNamed("Measure again")).toBeFalsy();
    expect(text()).not.toContain("describe the past");
  });

  it("offers measure again for a stale reading, and never a fast-forward", () => {
    render({ board: divergence("stale", { kind: "refreshBaseObservation", sessionId: "s", workspaceId: "w-1" }) });
    expect(buttonNamed("Measure again")).toBeTruthy();
    expect(buttonNamed("Fast-forward")).toBeFalsy();
    // All three tells: the stated tense, the dimmed numbers, and the changed action.
    expect(text()).toContain("These numbers describe the past");
    expect(text()).toContain("stale");
    expect(host.querySelector(".opacity-70")).toBeTruthy();
  });

  it("claims no numbers for an unmeasured reading", () => {
    render({
      board: board([
        fact({
          kind: "workspace_behind_base",
          severity: "attention",
          title: "Kyoto could not be measured against its base branch",
          detail: "No upstream or default branch ref is available to compare against.",
          target: { kind: "workspace", workspaceId: "w-1", sessionId: "s" },
          freshness: "unknown",
          action: { kind: "refreshBaseObservation", sessionId: "s", workspaceId: "w-1" },
        }),
      ]),
    });
    expect(text()).toContain("not measured");
    expect(buttonNamed("Measure again")).toBeTruthy();
    expect(buttonNamed("Fast-forward")).toBeFalsy();
    expect(text()).not.toContain("describe the past");
  });
});

describe("states", () => {
  it("shows skeleton rows rather than a spinner while reading", () => {
    render({ board: undefined });
    expect(host.querySelectorAll("li")).toHaveLength(3);
    expect(text()).not.toContain("Loading…");
    expect(host.querySelector(".animate-spin")).toBeFalsy();
  });

  it("says nothing needs you, without ceremony", () => {
    render({ board: board([]) });
    expect(text()).toContain("Nothing needs you");
    expect(text()).toContain("every workspace is close to its base");
  });

  it("explains that a failed read is safe to retry", () => {
    const onRefresh = vi.fn();
    render({ board: undefined, error: "database is locked", onRefresh });
    expect(text()).toContain("Work could not be read");
    expect(text()).toContain("this screen only reads");
    act(() => buttonNamed("Try again")?.click());
    expect(onRefresh).toHaveBeenCalledOnce();
  });

  it("keeps the fact and attaches the real reason when an action fails", async () => {
    const onAction = vi.fn(async (): Promise<WorkActionOutcome> => ({
      ok: false,
      reason: "The working tree has uncommitted changes, so this would not be a pure fast-forward.",
    }));
    render({
      board: board([
        fact({
          kind: "workspace_behind_base",
          severity: "attention",
          title: "Kyoto has drifted behind its base branch",
          target: { kind: "workspace", workspaceId: "w-1", sessionId: "s" },
          action: { kind: "refreshWorkspaceBase", sessionId: "s", workspaceId: "w-1" },
        }),
      ]),
      onAction,
    });
    await act(async () => {
      buttonNamed("Fast-forward")?.click();
    });
    // The fact is still true; only the attempt failed.
    expect(text()).toContain("Kyoto has drifted behind its base branch");
    expect(text()).toContain("uncommitted changes");
    expect(buttonNamed("Try again")).toBeTruthy();
    expect(host.querySelectorAll("li")).toHaveLength(1);
  });

  it("clears a previous failure when a retry succeeds", async () => {
    let fails = true;
    const onAction = vi.fn(async (): Promise<WorkActionOutcome> =>
      fails ? { ok: false, reason: "temporarily locked" } : { ok: true },
    );
    render({ board: board([fact()]), onAction });
    await act(async () => { buttonNamed("Review check")?.click(); });
    expect(text()).toContain("temporarily locked");
    fails = false;
    await act(async () => { buttonNamed("Try again")?.click(); });
    expect(text()).not.toContain("temporarily locked");
  });

  it("invokes the fact's own typed action", async () => {
    const onAction = vi.fn(async (): Promise<WorkActionOutcome> => ({ ok: true }));
    render({ board: board([fact()]), onAction });
    await act(async () => { buttonNamed("Review check")?.click(); });
    expect(onAction).toHaveBeenCalledWith({
      kind: "reviewCompletionCheck", sessionId: "s-1", attemptId: "a-1", checkId: "cargo-test",
    });
  });
});

describe("the suggested work notice", () => {
  it("is one quiet dismissible line when nothing is configured", () => {
    render({ board: board([fact()], { suggestions: { state: "not_configured", detail: null } }) });
    expect(text()).toContain("Suggested work is off until a briefing model is set up");
    const dismiss = host.querySelector("[aria-label='Dismiss the suggested work notice']") as HTMLButtonElement | null;
    expect(dismiss).toBeTruthy();
    act(() => dismiss?.click());
    expect(text()).not.toContain("Suggested work is off");
  });

  it("is absent when suggestions are not in the not-configured state", () => {
    render({ board: board([fact()], { suggestions: { state: "unavailable", detail: null } }) });
    expect(text()).not.toContain("Suggested work is off");
  });

  it("never offers a call to action for a feature that does not exist yet", () => {
    render({ board: board([fact()], { suggestions: { state: "not_configured", detail: null } }) });
    expect(text()).not.toMatch(/set up a model|configure|get started/i);
  });
});

describe("untrusted labels", () => {
  it("wraps a very long title rather than letting it overflow", () => {
    const long = "integration-tests-postgres-16-with-extensions-and-a-very-long-suite-name failed on feature/refactor-the-entire-billing-subsystem-take-three";
    render({ board: board([fact({ title: long })]) });
    expect(text()).toContain(long);
    const title = Array.from(host.querySelectorAll("p")).find(node => node.textContent?.includes(long));
    expect(title?.className).toContain("[overflow-wrap:anywhere]");
  });
});

describe("keyboard", () => {
  it("makes each row's action the only tab stop in the row", () => {
    render({
      board: board([
        fact({ dedupeKey: "1" }),
        fact({ dedupeKey: "2", severity: "attention", title: "second" }),
      ]),
    });
    // The rows themselves are not buttons: nothing is clickable but unmarked.
    for (const row of Array.from(host.querySelectorAll("li"))) {
      expect(row.getAttribute("tabindex")).toBeNull();
      expect(row.tagName).toBe("LI");
      expect(row.querySelectorAll("button")).toHaveLength(1);
    }
  });

  it("disables an action while it is running, so it cannot be fired twice", async () => {
    let release: (value: WorkActionOutcome) => void = () => {};
    const onAction = vi.fn(() => new Promise<WorkActionOutcome>(resolve => { release = resolve; }));
    render({ board: board([fact()]), onAction });
    await act(async () => { buttonNamed("Review check")?.click(); });
    expect(buttonNamed("Review check")?.disabled).toBe(true);
    await act(async () => { release({ ok: true }); });
    expect(buttonNamed("Review check")?.disabled).toBe(false);
    expect(onAction).toHaveBeenCalledOnce();
  });
});
