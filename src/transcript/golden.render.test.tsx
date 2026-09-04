// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "../components/AgentConversation";
import { normalizeAgentEvent } from "./codec";
import { durableEntries, HARNESSES, harnessStream, type GoldenHarness } from "./golden";
import type { AgentEvent, SessionEntry, Session } from "../types";

/**
 * The last mile: four harnesses, one rendered transcript.
 *
 * `golden.test.ts` proves the reduction agrees. This proves the component above
 * it draws the same thing without knowing which agent produced the turn.
 *
 * **Compared across harnesses, never against a literal.** A fixed expected list
 * would pin today's grouping, and grouping changes; a change that treated
 * every harness identically would then fail here for no reason, and the
 * temptation would be to edit the literal rather than to look. Cross-harness
 * equality is the claim this test actually wants to make, and it survives the
 * rewrite. What the digest does pin is the part that is a contract rather than
 * a layout: that a thought is a thought, in a named state, with the one thinking
 * mark on it while it streams.
 */

const session = (harness: GoldenHarness): Session => ({
  id: harness, workspaceId: "w", harness, label: "Orchestrator", status: "idle",
  startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null,
  metricSource: "reported", model: null, restorationMode: "fresh",
  continuationFidelity: "native", kind: "orchestrator",
});

/**
 * Whether a fold is showing its contents.
 *
 * Read off the toggle's own `aria-expanded` rather than off what sits after it.
 * Neighbours are not a reliable answer: a finished group renders a trailer
 * saying how long the work took, and a live collapsed one renders the step it
 * is running right now, both between the button and the disclosure. The
 * attribute is what the button tells a screen reader, so it is the same answer
 * a reader gets.
 */
function opened(toggle: Element): boolean {
  return toggle.getAttribute("aria-expanded") === "true";
}

/**
 * One row, as a reader would describe it: whose it is, what state it is in,
 * whether it is folded. Deliberately not the markup, and deliberately not the
 * prose — provider wording differs and is allowed to.
 */
function rowDigest(row: Element): string {
  // The group is asked first. A run now carries the thoughts that fell inside
  // it, drawn by the same `Reasoning` component and so wearing the same
  // `data-thinking` marker; asking about the thought first would report a
  // hundred-step run as a thought.
  const toggle = [...row.querySelectorAll("button")].find(button => /\bsteps?\b/.test(button.textContent ?? ""));
  if (toggle) {
    const steps = /(\d+)\s+steps?/.exec(toggle.textContent ?? "")?.[1] ?? "?";
    return `activity-group:${steps}:${opened(toggle) ? "open" : "closed"}`;
  }
  const thought = row.querySelector("[data-thinking]");
  if (thought) {
    const state = thought.getAttribute("data-thinking");
    if (state === "streaming") {
      // The mark is part of the claim: every harness's streaming thought draws
      // the same one, from the same component.
      return `thought:streaming:${row.querySelector(".thinking-shimmer") ? "shimmer" : "no-mark"}`;
    }
    return `thought:completed:${(thought as HTMLDetailsElement).open ? "open" : "closed"}`;
  }
  if (row.querySelector('[class*="max-w-[85%]"]')) return "user-bubble";
  if (row.querySelector(".thinking-shimmer")) return "assistant-prose:pending";
  return "assistant-prose";
}

async function render(
  harness: GoldenHarness,
  source: { events?: AgentEvent[]; forestEntries?: SessionEntry[] },
): Promise<{ digest: string[]; unmount: () => Promise<void> }> {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(
    <AgentConversation
      session={session(harness)}
      onResolve={() => undefined}
      events={source.events ?? []}
      forestEntries={source.forestEntries}
      activeLeafId={source.forestEntries?.at(-1)?.id ?? null}
    />,
  ));
  const rows = container.querySelector(".max-w-3xl");
  return {
    digest: [...(rows?.children ?? [])].map(rowDigest),
    unmount: async () => { await act(async () => root.unmount()); container.remove(); },
  };
}

async function digestOf(harness: GoldenHarness, source: { events?: AgentEvent[]; forestEntries?: SessionEntry[] }): Promise<string[]> {
  const { digest, unmount } = await render(harness, source);
  await unmount();
  return digest;
}

/**
 * The turn as far as its first thought, before that thought has finished.
 *
 * Cut by normalized type rather than by wire kind: normalization is one event
 * in, one event out, so the index of the first streamed thought in the
 * normalized list is its index in the raw one.
 */
function untilFirstThought(harness: GoldenHarness): AgentEvent[] {
  const events = harnessStream(harness);
  const firstThought = events.map(normalizeAgentEvent).findIndex(event => event.type === "thinking.delta");
  expect(firstThought, `${harness} streams no thought at all`).toBeGreaterThanOrEqual(0);
  return events.slice(0, firstThought + 1);
}

describe("golden streams, rendered", () => {
  it("draws the same transcript for every harness", async () => {
    const drawn: Record<string, string[]> = {};
    for (const harness of HARNESSES) drawn[harness] = await digestOf(harness, { events: harnessStream(harness) });

    for (const harness of HARNESSES) {
      expect(drawn[harness], `${harness} drew a different transcript`).toEqual(drawn[HARNESSES[0]]);
    }
    // Not a snapshot of the grouping, which is free to change: the shape of a
    // turn. It opens with what the reader asked for, ends with the answer, and
    // has thinking and tool work in between.
    const shape = drawn[HARNESSES[0]];
    expect(shape.at(0)).toBe("user-bubble");
    expect(shape.at(-1)).toBe("assistant-prose");
    expect(shape.filter(row => row.startsWith("thought:")).length).toBeGreaterThan(0);
    expect(shape.filter(row => row.startsWith("activity-group:")).length).toBeGreaterThan(0);
    // A settled thought is collapsed. (A finished group is too, unless it holds
    // a patch, which opens itself on purpose: a diff the reader has to go
    // digging for is not an inline diff.)
    for (const row of shape) {
      if (row.startsWith("thought:")) expect(row).toBe("thought:completed:closed");
    }
  });

  it("draws one streaming thought mid-turn, whoever is thinking", async () => {
    const drawn: Record<string, string[]> = {};
    for (const harness of HARNESSES) drawn[harness] = await digestOf(harness, { events: untilFirstThought(harness) });

    for (const harness of HARNESSES) {
      expect(drawn[harness], `${harness} draws a thought in flight differently`).toEqual(drawn[HARNESSES[0]]);
      // Same component, same state, same mark: the one thing a reader must not
      // be able to use to tell the agents apart.
      expect(drawn[harness]).toContain("thought:streaming:shimmer");
    }
  });

  it("draws the durable projection the way it drew the live one", async () => {
    for (const harness of HARNESSES) {
      const live = await digestOf(harness, { events: harnessStream(harness) });
      const durable = await digestOf(harness, { forestEntries: durableEntries(harness) });
      expect(durable, `${harness} replays as a different transcript than it streamed`).toEqual(live);
    }
  });
});
