// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkBoard, WorkFact, WorkFactAction, WorkTask } from "../protocol/generated/protocol";
import { WorkView, type WorkActionOutcome } from "./WorkView";
import type { TaskAction } from "./workTasks";

const RETRY_LABEL_TEXT = "Try again";

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
    generatedAt: "2026-08-19T12:00:00.000Z",
    sources: [],
    settings: {
      briefing: null,
      enabledConnectorInstances: [],
      refreshOnFocus: false,
      refreshIntervalMinutes: null,
      cooldownMinutes: 15,
      limits: { maxWallSeconds: 600, maxTurns: 12, maxToolCalls: 24, maxOutputTokens: null, costCeilingMicrousd: null },
    },
    suggestions: { state: "provider_unsupported", detail: null },
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
    render({ board: board([fact()], { suggestions: { state: "provider_unsupported", detail: null } }) });
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

describe("counting", () => {
  it("counts the same set the rail badge counts", () => {
    // The header and the badge must not disagree about what is waiting on you.
    render({
      board: board([
        fact({ dedupeKey: "b", severity: "blocking" }),
        fact({ dedupeKey: "a", severity: "attention", title: "second" }),
        fact({ dedupeKey: "i", severity: "info", title: "third" }),
      ]),
    });
    expect(text()).toContain("2 things need you");
  });

  it("does not say anything needs you when only info is on the board", () => {
    render({ board: board([fact({ severity: "info", title: "worth knowing" })]) });
    expect(text()).toContain("Nothing is waiting on you");
    expect(text()).toContain("worth knowing, not urgent");
    // …and still shows the row, which counting only urgency must not hide.
    expect(host.querySelectorAll("li")).toHaveLength(1);
    expect(text()).not.toContain("Nothing needs you");
  });

  it("says one thing in the singular", () => {
    render({ board: board([fact()]) });
    expect(text()).toContain("1 thing needs you");
  });
});

describe("a failed re-read", () => {
  it("keeps the board and says what is on screen is the last thing read", () => {
    // A read that failed adds no information, so replacing the board with an error
    // panel would lose what the reader had.
    render({ board: board([fact()]), refreshError: "database is locked" });
    expect(text()).toContain("Could not re-read the board");
    expect(text()).toContain("database is locked");
    expect(text()).toContain("the last thing Bridge read");
    expect(text()).toContain("cargo-test failed on Kyoto");
    expect(text()).not.toContain("Work could not be read");
  });

  it("shows the full panel only when there is no board at all", () => {
    render({ board: undefined, error: "database is locked" });
    expect(text()).toContain("Work could not be read");
    expect(host.querySelectorAll("li")).toHaveLength(0);
  });

  it("prefers the board over an error that arrived with one", () => {
    render({ board: board([fact()]), error: "stale error from an earlier read" });
    expect(host.querySelectorAll("li")).toHaveLength(1);
    expect(text()).not.toContain("Work could not be read");
  });
});

describe("double clicks", () => {
  it("fires an action once even when clicked twice in the same tick", async () => {
    // `disabled` is React state and is not in effect until the next render, so two
    // clicks in one tick would both get through without a synchronous guard. A second
    // fast-forward is not harmless.
    let release: (value: WorkActionOutcome) => void = () => {};
    const onAction = vi.fn(() => new Promise<WorkActionOutcome>(resolve => { release = resolve; }));
    render({ board: board([fact()]), onAction });
    const button = buttonNamed("Review check")!;
    await act(async () => {
      button.click();
      button.click();
    });
    expect(onAction).toHaveBeenCalledOnce();
    await act(async () => { release({ ok: true }); });
  });

  it("re-enables the button when the handler throws instead of returning an outcome", async () => {
    // A button stuck disabled forever is the worst way to learn the handler broke.
    const onAction = vi.fn(async (): Promise<WorkActionOutcome> => { throw new Error("handler exploded"); });
    render({ board: board([fact()]), onAction });
    await act(async () => { buttonNamed("Review check")?.click(); });
    const retry = buttonNamed(RETRY_LABEL_TEXT)!;
    expect(retry.disabled).toBe(false);
    expect(text()).toContain("handler exploded");
  });
});

describe("accessibility", () => {
  it("is a named region, so it can be jumped to", () => {
    render();
    expect(host.querySelector("section[aria-label='Work']")).toBeTruthy();
  });

  it("names each band's list, so a row is heard in context", () => {
    render({
      board: board([
        fact({ dedupeKey: "b", severity: "blocking" }),
        fact({ dedupeKey: "a", severity: "attention", title: "second" }),
      ]),
    });
    expect(host.querySelector("ul[aria-label='Blocking work']")).toBeTruthy();
    expect(host.querySelector("ul[aria-label='Attention work']")).toBeTruthy();
  });

  it("announces the count politely rather than interrupting", () => {
    render();
    const live = host.querySelector("[aria-live='polite']");
    expect(live).toBeTruthy();
    expect(live?.textContent).toContain("needs you");
  });

  it("points a failed action's button at the reason", async () => {
    const onAction = vi.fn(async (): Promise<WorkActionOutcome> => ({ ok: false, reason: "locked" }));
    render({ board: board([fact()]), onAction });
    await act(async () => { buttonNamed("Review check")?.click(); });
    const button = buttonNamed("Try again")!;
    const describedBy = button.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    // getElementById rather than a selector: a dedupe key contains colons, which a
    // CSS id selector would need escaped, and CSS.escape is absent in this jsdom.
    expect(document.getElementById(describedBy!)?.textContent).toContain("locked");
  });

  it("does not point at a reason when there is none", () => {
    render();
    expect(buttonNamed("Review check")?.getAttribute("aria-describedby")).toBeNull();
  });

  it("hides every decorative mark from the accessible tree", () => {
    render();
    // The severity edge, the source tile and the freshness dot all carry meaning that
    // is stated in words elsewhere, so none of them should be announced twice.
    const decorative = host.querySelectorAll("[aria-hidden='true']");
    expect(decorative.length).toBeGreaterThanOrEqual(3);
    for (const node of Array.from(host.querySelectorAll("svg"))) {
      const hidden = node.getAttribute("aria-hidden") === "true" || node.closest("[aria-hidden='true']");
      expect(hidden).toBeTruthy();
    }
  });
});

describe("narrow widths", () => {
  it("gives the action its own row under a hairline below the sm breakpoint", () => {
    // At 420px a button beside a wrapping title leaves neither enough room, so the
    // action drops. Asserted on the classes because jsdom has no layout.
    render();
    const action = buttonNamed("Review check")!.parentElement!;
    expect(action.className).toContain("w-full");
    expect(action.className).toContain("border-t");
    expect(action.className).toContain("sm:w-auto");
    expect(action.className).toContain("sm:border-0");
  });

  it("lets the row wrap below sm and not above it", () => {
    render();
    const row = host.querySelector("li")!;
    expect(row.className).toContain("flex-wrap");
    expect(row.className).toContain("sm:flex-nowrap");
  });
});

describe("suggested work", () => {
  const suggested = (overrides: Partial<WorkTask> = {}): WorkTask => ({
    id: "task-v1:abc",
    fingerprint: "v1:abc",
    connectorInstanceId: "slack-work",
    canonicalResourceId: "slack:slack-work:1.1",
    sourceKind: "slack.message",
    title: "Reply to Priya",
    why: "She asked twice and nobody answered.",
    rank: 1,
    confidenceBps: 8_200,
    state: "active",
    pinned: false,
    snoozedUntil: null,
    evidenceDigest: "d".repeat(64),
    evidenceTarget: { kind: "externalLink", url: "https://app.slack.com/archives/C1/p1", host: "app.slack.com" },
    evidenceObservedAt: "2026-08-19T11:59:00.000Z",
    missCount: 0,
    workspaceId: null,
    createdAt: "2026-08-19T11:00:00.000Z",
    updatedAt: "2026-08-19T11:00:00.000Z",
    ...overrides,
  });

  const withTasks = (tasks: WorkTask[], facts: WorkFact[] = []) => board(facts, { tasks });

  it("renders a suggested band below the facts, or none at all", () => {
    render({ board: withTasks([suggested()]), onTaskAction: ok as never });
    expect(text()).toContain("Suggested");
    expect(text()).toContain("Reply to Priya");
    expect(text()).toContain("Slack · slack-work");
    expect(text()).toContain("high confidence");
    // No tasks, no section — not an empty band inviting setup.
    render({ board: board([fact()]) });
    expect(text()).not.toContain("from your connected tools");
  });

  it("offers restore for a hidden snoozed task when the user reveals it", async () => {
    render({ board: withTasks([suggested({ state: "snoozed", pinned: true })]), onTaskAction: ok as never });
    expect(host.querySelectorAll("ul[aria-label='Suggested work'] li")).toHaveLength(0);
    expect(buttonNamed("Restore")).toBeFalsy();
    await act(async () => { buttonNamed("Show hidden")?.click(); });
    expect(host.querySelectorAll("ul[aria-label='Hidden suggested work'] li")).toHaveLength(1);
    expect(buttonNamed("Restore")).toBeTruthy();

    render({ board: withTasks([suggested({ state: "active" })]), onTaskAction: ok as never });
    for (const label of ["Start", "Done", "Snooze", "Dismiss"]) {
      expect(buttonNamed(label), label).toBeTruthy();
    }
    expect(buttonNamed("Restore")).toBeFalsy();
  });

  it("hides a stale task unless it is pinned", () => {
    render({ board: withTasks([suggested({ state: "stale" })]), onTaskAction: ok as never });
    expect(host.querySelectorAll("ul[aria-label='Suggested work'] li")).toHaveLength(0);
    render({ board: withTasks([suggested({ state: "stale", pinned: true })]), onTaskAction: ok as never });
    expect(host.querySelectorAll("ul[aria-label='Suggested work'] li")).toHaveLength(1);
    expect(text()).toContain("Stale");
    expect(text()).toContain("Pinned");
  });

  it("invokes the action it was asked for and keeps the row when it fails", async () => {
    // Typed with the real signature, so asserting on the arguments is possible at all —
    // an inferred zero-parameter mock makes `calls[0][1]` a type error, which vitest would
    // never have told me about.
    const onTaskAction = vi.fn(
      async (_task: WorkTask, _action: TaskAction): Promise<WorkActionOutcome> => ({
        ok: false,
        reason: "a snoozed task cannot be snoozed",
      }),
    );
    render({ board: withTasks([suggested()]), onTaskAction });
    await act(async () => { buttonNamed("Dismiss")?.click(); });
    expect(onTaskAction).toHaveBeenCalledOnce();
    expect(onTaskAction.mock.calls[0][1]).toBe("dismiss");
    expect(text()).toContain("a snoozed task cannot be snoozed");
    expect(text()).toContain("Reply to Priya");
  });

  it("keeps pinning separate from the state actions", async () => {
    const onTogglePin = vi.fn(async (): Promise<WorkActionOutcome> => ({ ok: true }));
    const onTaskAction = vi.fn(async (): Promise<WorkActionOutcome> => ({ ok: true }));
    render({ board: withTasks([suggested()]), onTaskAction, onTogglePin });
    const pin = host.querySelector("[aria-label='Pin this task']") as HTMLButtonElement;
    expect(pin.getAttribute("aria-pressed")).toBe("false");
    await act(async () => { pin.click(); });
    expect(onTogglePin).toHaveBeenCalledOnce();
    expect(onTaskAction).not.toHaveBeenCalled();
  });

  it("names the host before you click through to it", () => {
    render({ board: withTasks([suggested()]), onOpenEvidence: () => {} });
    expect(buttonNamed("Open on app.slack.com")).toBeTruthy();
  });

  it("draws no evidence affordance for a target that would be refused", () => {
    render({
      board: withTasks([suggested({ evidenceTarget: { kind: "externalLink", url: "http://app.slack.com/x", host: "app.slack.com" } })]),
      onOpenEvidence: () => {},
    });
    expect(text()).not.toContain("Open on");
  });

  it("fires an action once even when clicked twice in the same tick", async () => {
    let release: (value: WorkActionOutcome) => void = () => {};
    const onTaskAction = vi.fn(() => new Promise<WorkActionOutcome>(resolve => { release = resolve; }));
    render({ board: withTasks([suggested()]), onTaskAction });
    const button = buttonNamed("Done")!;
    await act(async () => { button.click(); button.click(); });
    expect(onTaskAction).toHaveBeenCalledOnce();
    await act(async () => { release({ ok: true }); });
  });

  it("shows a board of only suggested work rather than the empty panel", () => {
    render({ board: withTasks([suggested()]), onTaskAction: ok as never });
    expect(text()).not.toContain("Nothing needs you");
  });
});
