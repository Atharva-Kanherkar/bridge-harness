// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "../components/AgentConversation";
import { HARNESSES, harnessStream, type GoldenHarness } from "./golden";
import type { Session } from "../types";

/**
 * The last mile: four harnesses, one rendered transcript.
 *
 * `golden.test.ts` proves the reduction agrees. This proves the component
 * above it draws the same thing — same rows, in the same order, with the same
 * shapes — without knowing which agent produced the turn.
 */

const session = (harness: GoldenHarness): Session => ({
  id: harness, workspaceId: "w", harness, label: "Orchestrator", status: "idle",
  startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null,
  metricSource: "reported", model: null, restorationMode: "fresh",
  continuationFidelity: "native", kind: "orchestrator",
});

/**
 * A row's shape, as a reader would describe it: whose bubble it is, whether it
 * is a thought, whether it is a folded run of tool work. Deliberately not the
 * markup — provider prose differs, and it is allowed to.
 */
function rowShape(row: Element): string {
  const buttons = [...row.querySelectorAll("button")];
  if (buttons.some(button => /\bsteps?\b/.test(button.textContent ?? ""))) return "activity-group";
  if ((row.textContent ?? "").includes("Thought for")) return "thought";
  if (row.querySelector('[class*="max-w-[85%]"]')) return "user-bubble";
  return "assistant-prose";
}

async function renderHarness(harness: GoldenHarness): Promise<{ shapes: string[]; unmount: () => Promise<void> }> {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(
    <AgentConversation session={session(harness)} onResolve={() => undefined} events={harnessStream(harness)}/>,
  ));
  const rows = container.querySelector(".max-w-3xl");
  const shapes = [...(rows?.children ?? [])].map(rowShape);
  return {
    shapes,
    unmount: async () => { await act(async () => root.unmount()); container.remove(); },
  };
}

describe("golden streams, rendered", () => {
  it("draws the same transcript for every harness", async () => {
    const drawn: Record<string, string[]> = {};
    for (const harness of HARNESSES) {
      const { shapes, unmount } = await renderHarness(harness);
      drawn[harness] = shapes;
      await unmount();
    }
    // One turn: the user's bubble, a thought, the command, a second thought,
    // then the read and the patch folded into one run of tool work, then the
    // reply. The two tool rows after the second thought are consecutive, so
    // they group; the command stands alone because a thought interrupts it.
    expect(drawn.claude).toEqual([
      "user-bubble", "thought", "activity-group", "thought", "activity-group", "assistant-prose",
    ]);
    for (const harness of HARNESSES) {
      expect(drawn[harness], `${harness} drew a different transcript`).toEqual(drawn.claude);
    }
  });
});
