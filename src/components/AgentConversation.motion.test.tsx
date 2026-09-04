// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session } from "../types";

// The static suite covers what the transcript renders. This one covers how it
// moves: what stays mounted while it leaves, and what is swapped rather than
// overwritten. `skipAnimations` collapses the frames, so every assertion is
// about lifecycle and structure — never about a computed style mid-flight.

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

const command = (overrides: Partial<AgentEvent> = {}) => event(1, "command.completed", {
  title: "bun run test",
  data: { type: "commandExecution", command: "bun run test", durationMs: 3000, aggregatedOutput: "92 pass\n0 fail" },
  ...overrides,
});

let host: HTMLDivElement;
let root: Root;

function mount(events: AgentEvent[], props: Record<string, unknown> = {}) {
  act(() => {
    root.render(<AgentConversation session={session} events={events} onResolve={() => {}} {...props} />);
  });
}

/** Let Framer's frame loop run so finished exits actually unmount. */
async function settle() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 40));
  });
}

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

describe("transcript entrances", () => {
  it("drives row entrances from Framer rather than the CSS class", () => {
    mount([
      event(1, "message.completed", { role: "user", text: "Fix the drift measurement" }),
      event(2, "message.completed", { role: "assistant", text: "On it." }),
    ]);
    expect(host.textContent).toContain("On it.");
    expect(host.querySelectorAll(".chat-message-enter")).toHaveLength(0);
  });

  it("keeps the rows already on screen when one more arrives", () => {
    const first = event(1, "message.completed", { role: "user", text: "Fix the drift measurement" });
    mount([first]);
    const before = host.querySelector('[class*="ml-auto"]');
    expect(before).not.toBeNull();

    mount([first, event(2, "message.completed", { role: "assistant", text: "On it." })]);
    // Same node, not a remount: an arriving row must not restart the entrance of
    // what is already there.
    expect(host.querySelector('[class*="ml-auto"]')).toBe(before);
  });
});

describe("tool row disclosure", () => {
  async function openGroup() {
    mount([command()]);
    const summary = buttonWith("Ran 1 command");
    expect(summary).toBeDefined();
    act(() => summary!.click());
    await settle();
  }

  it("animates a tool row's output open and holds it mounted while it collapses", async () => {
    await openGroup();
    const row = buttonWith("bun run test");
    expect(row).toBeDefined();

    act(() => row!.click());
    expect(host.textContent).toContain("92 pass");

    act(() => row!.click());
    // Held for its height transition rather than snapping shut.
    expect(host.textContent).toContain("92 pass");
    await settle();
    expect(host.textContent).not.toContain("92 pass");
  });

  it("animates an activity group closed the same way", async () => {
    await openGroup();
    expect(host.textContent).toContain("bun run test");

    const summary = buttonWith("Ran 1 command");
    act(() => summary!.click());
    expect(host.textContent).toContain("bun run test");
    await settle();
    expect(host.textContent).not.toContain("bun run test");
  });

  it("shows exactly one status glyph, swapped when the run finishes", async () => {
    // A live run stays collapsed and names the step it is on, so the one
    // spinner on screen is the current step's, not a row's.
    mount([command({ status: "inProgress" })]);
    await settle();
    expect(host.querySelectorAll(".animate-spin")).toHaveLength(1);

    // Opened by hand, the finished call wears the tick instead.
    mount([command({ status: "completed" })]);
    await settle();
    const summary = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes("Ran 1 command"));
    await act(async () => summary!.click());
    await settle();
    expect(host.querySelectorAll(".animate-spin")).toHaveLength(0);
    expect(host.querySelectorAll(".text-success").length).toBeGreaterThan(0);
  });
});

describe("session switches and optimistic bubbles", () => {
  it("replaces the transcript atomically on a session switch instead of animating every old row out", async () => {
    // Real durations here: the defect this guards is *timing-shaped*. Under
    // skipped animations an unkeyed presence also drops its ghosts between
    // acts, and the regression would be invisible.
    mount([event(1, "message.completed", { role: "user", text: "First session message" })]);
    const next = { ...session, id: "s2" };
    MotionGlobalConfig.skipAnimations = false;
    try {
      // A synchronous switch: the old transcript must not linger through exit
      // frames — keyed presence replaces the tree in one commit. Distinct
      // item ids, so the incoming row is genuinely a different child.
      mount([event(42, "message.completed", { role: "user", text: "Second session message" })], { session: next });
      expect(host.textContent).not.toContain("First session message");
      expect(host.textContent).toContain("Second session message");
    } finally {
      MotionGlobalConfig.skipAnimations = true;
    }
    await settle();
    expect(host.textContent).not.toContain("First session message");
  });

  it("keeps a later optimistic bubble in place when an earlier one lands as real", async () => {
    const bubblesWith = (text: string) =>
      [...host.querySelectorAll<HTMLElement>('[class*="ml-auto"]')].filter(node => node.textContent === text);
    mount([], { working: true, pendingMessages: ["alpha", "beta"] });
    const beta = bubblesWith("beta");
    expect(beta).toHaveLength(1);

    // "alpha" becomes a real message; its ghost leaves, but "beta" must not
    // move, flip its text, or be the thing that fades.
    mount([event(1, "message.completed", { role: "user", text: "alpha" })], { pendingMessages: ["beta"] });
    expect(bubblesWith("beta")).toHaveLength(1);
    expect(bubblesWith("beta")[0]).toBe(beta[0]);
    // The leaving ghost is the resolved "alpha" — the real message plus its
    // fading twin, until the exit lands.
    expect(bubblesWith("alpha")).toHaveLength(2);
    await settle();
    expect(bubblesWith("alpha")).toHaveLength(1);
    expect(bubblesWith("beta")).toHaveLength(1);
  });
});

describe("alerts and approvals", () => {  it("gives an error item the alert role and its described title", () => {
    mount([event(1, "error", { status: "failed", text: "Adapter exited with status 1" })]);
    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toContain("Adapter exited");
  });

  it("swaps an approval's actions for its outcome when it resolves", async () => {
    const request = event(1, "approval.requested", {
      title: "Push branch", status: "pending", data: { command: "git push" },
    });
    mount([request]);
    expect(buttonWith("Allow once")).toBeDefined();

    mount([request, event(2, "approval.resolved", { data: { requestEventId: 1, decision: "accept" } })]);
    await settle();
    expect(buttonWith("Allow once")).toBeUndefined();
    expect(host.textContent).toContain("Allowed once");
  });

  it("still resolves through the callback it is given", () => {
    const onResolve = vi.fn();
    mount([event(1, "approval.requested", { title: "Push branch", status: "pending", data: { command: "git push" } })], { onResolve });
    act(() => buttonWith("Allow once")!.click());
    expect(onResolve).toHaveBeenCalledWith(1, "accept");
  });
});
