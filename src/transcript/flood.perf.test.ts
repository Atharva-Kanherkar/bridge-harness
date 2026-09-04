import { describe, expect, it } from "vitest";
import { normalizeAgentEvent } from "./codec";
import { groupItems } from "./grouping";
import { reduceTranscript } from "./reducer";
import { floodStream, FLOOD_STEPS } from "./fixtures/codexFlood";

/**
 * A guard, not a benchmark.
 *
 * The defect this branch fixes was not a slow function; it was quadratic
 * structure — every 50 ms flush reduced the whole turn, grouped it into a row
 * per step, and reconciled every one of those rows. Two assertions catch a
 * return to that: the transcript's *shape* stays bounded as steps pile up, and
 * one flush's worth of work stays far inside a budget a loaded CI runner can
 * still meet.
 *
 * The budget is deliberately loose. A tight timing assertion on a shared
 * runner is a flaky test, and a flaky test gets deleted; 50 ms is roughly
 * twenty times what this costs on a developer machine, so it fails only if the
 * work has changed order, not if the machine is busy.
 */

const BUDGET_MS = 50;
const PASSES = 5;

function flushWork(steps: number) {
  const events = floodStream(steps).map(normalizeAgentEvent);
  return () => groupItems(reduceTranscript(events));
}

function medianMs(work: () => unknown, passes = PASSES): number {
  const samples: number[] = [];
  for (let pass = 0; pass < passes; pass += 1) {
    const started = performance.now();
    work();
    samples.push(performance.now() - started);
  }
  return samples.sort((left, right) => left - right)[Math.floor(passes / 2)];
}

describe("a hundred-step turn, reduced and grouped", () => {
  it("stays bounded in top-level rows as the steps pile up", () => {
    // Five rows: the user's message, the thought the turn opened with, the
    // run, the thought it closed with, the reply. The count must not move
    // with the step count — that is the whole claim.
    for (const steps of [10, 100, 400, 1000]) {
      const rows = groupItems(reduceTranscript(floodStream(steps).map(normalizeAgentEvent)));
      expect(rows, `${steps} steps drew ${rows.length} top-level rows`).toHaveLength(5);
    }
  });

  it("puts every call in the one group rather than losing any", () => {
    const rows = groupItems(reduceTranscript(floodStream().map(normalizeAgentEvent)));
    const group = rows.find(row => row.kind === "group");
    expect(group?.kind).toBe("group");
    // A hundred calls plus the ninety-nine thoughts that fell between them.
    expect(group && group.kind === "group" && group.items).toHaveLength(FLOOD_STEPS * 2 - 1);
  });

  it("does one flush's work well inside the budget", () => {
    const elapsed = medianMs(flushWork(FLOOD_STEPS));
    expect(elapsed, `reduce + group of ${FLOOD_STEPS} steps took ${elapsed.toFixed(1)}ms`).toBeLessThan(BUDGET_MS);
  });

  it("grows with the turn rather than with the square of it", () => {
    // Four times the steps must not cost sixteen times the work. Generous
    // because the machine is shared: this catches a per-item scan over the
    // whole list, which is what a return to the old grouping would look like.
    const small = Math.max(medianMs(flushWork(100)), 0.05);
    const large = medianMs(flushWork(400));
    expect(large / small, `400 steps cost ${(large / small).toFixed(1)}x what 100 did`).toBeLessThan(10);
  });
});
