// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session } from "../types";

/**
 * What a flush costs.
 *
 * A live turn arrives at twenty flushes a second, and every flush hands the
 * transcript a freshly folded copy of every row — the reducer is a fold, so
 * reference equality never holds. Before the rows were memoized on their own
 * signature, that meant re-rendering the entire transcript, prose and diffs
 * included, twenty times a second for the length of a hundred-step turn.
 *
 * Counting renders needs a component that reports them, so the markdown
 * renderer is replaced with one that does. It is the leaf of every prose row
 * and of every thought, which makes it a faithful proxy for "this row
 * re-rendered".
 */

const renders = vi.hoisted(() => ({ markdown: 0, mention: 0 }));

vi.mock("./Markdown", async importOriginal => {
  const actual = await importOriginal<typeof import("./Markdown")>();
  return {
    ...actual,
    Markdown: ({ text }: { text: string }) => { renders.markdown += 1; return <div data-markdown="">{text}</div>; },
    MentionText: ({ text }: { text: string }) => { renders.mention += 1; return <span>{text}</span>; },
  };
});

const { AgentConversation } = await import("./AgentConversation");

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

/** A settled turn: a question, a thought, a command still running, an answer. */
const TURN: AgentEvent[] = [
  event(1, "message.completed", { itemId: "u-1", role: "user", text: "Port the store." }),
  event(2, "reasoning.completed", { itemId: "r-1", text: "Start with the suite." }),
  event(3, "command.started", { itemId: "c-1", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
  event(4, "message.completed", { itemId: "a-1", role: "assistant", text: "Working on it." }),
];

let host: HTMLDivElement;
let root: Root;

function mount(events: AgentEvent[]) {
  act(() => {
    root.render(<AgentConversation session={session} events={events} onResolve={() => {}} />);
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  renders.markdown = 0;
  renders.mention = 0;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

describe("a flush that changes one row", () => {
  it("leaves every other row alone", () => {
    mount(TURN);
    expect(renders.markdown).toBeGreaterThan(0);
    const before = { ...renders };

    // One more chunk of the running command's output: a new events array, a
    // new reduce, a new set of item objects — and nothing else that changed.
    mount([...TURN, event(0, "command.output_delta", { itemId: "c-1", sequence: 0, status: "inProgress", text: "1 failing\n" })]);
    expect(renders.markdown).toBe(before.markdown);
    expect(renders.mention).toBe(before.mention);
  });

  it("still redraws the row that did change", () => {
    mount(TURN);
    const before = renders.markdown;
    mount([
      ...TURN.slice(0, 3),
      event(4, "message.completed", { itemId: "a-1", role: "assistant", text: "Working on it. Done." }),
    ]);
    expect(renders.markdown).toBeGreaterThan(before);
    expect(host.textContent).toContain("Working on it. Done.");
  });

  it("does not re-render anything at all when the input is unchanged", () => {
    // Same array, same reference: the projections and the grouping sit behind
    // `useMemo`, so a parent re-render costs nothing.
    mount(TURN);
    const before = { ...renders };
    mount(TURN);
    expect(renders.markdown).toBe(before.markdown);
    expect(renders.mention).toBe(before.mention);
  });
});
