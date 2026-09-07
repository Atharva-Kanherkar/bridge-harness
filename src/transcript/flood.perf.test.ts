import { cpuUsage } from "node:process";
import { describe, expect, it } from "vitest";
import { normalizeAgentEvent } from "./codec";
import { groupItems } from "./grouping";
import { reduceTranscript } from "./reducer";
import { floodStream, FLOOD_STEPS } from "./fixtures/codexFlood";

/**
 * A guard, not a benchmark.
 *
 * The regression this guards against is quadratic structure: every 50 ms
 * flush reduced the whole turn, grouped it into a row
 * per step, and reconciled every one of those rows. Two assertions catch a
 * return to that: the transcript's *shape* stays bounded as steps pile up, and
 * one flush's CPU work stays far inside its budget.
 *
 * The budget is deliberately loose. A tight timing assertion on a shared
 * runner is a flaky test, and a flaky test gets deleted; 50 ms is roughly
 * twenty times what this costs on a developer machine, so it fails only if the
 * work has changed order, not if the machine is busy. Measure this worker's
 * CPU time: wall time includes pauses while other test workers run. Those
 * pauses made an otherwise linear 1000-step flush appear over 30 times slower
 * than a sub-millisecond 100-step flush.
 */

const BUDGET_MS = 50;
const PASSES = 5;
const WARMUP_PASSES = 3;
const BATCH_SIZE = 10;

function flushWork(steps: number) {
  const events = floodStream(steps).map(normalizeAgentEvent);
  return () => groupItems(reduceTranscript(events));
}

function sampleMs(work: () => unknown): number {
  const started = cpuUsage();
  for (let run = 0; run < BATCH_SIZE; run += 1) work();
  const elapsed = cpuUsage(started);
  return (elapsed.user + elapsed.system) / 1000 / BATCH_SIZE;
}

function median(samples: number[]): number {
  return samples.sort((left, right) => left - right)[Math.floor(samples.length / 2)];
}

function medianMs(work: () => unknown): number {
  for (let pass = 0; pass < WARMUP_PASSES; pass += 1) work();
  return median(Array.from({ length: PASSES }, () => sampleMs(work)));
}

function growthRatio(smallWork: () => unknown, largeWork: () => unknown): number {
  for (let pass = 0; pass < WARMUP_PASSES; pass += 1) {
    smallWork();
    largeWork();
  }
  // Pair the sizes and alternate their order so JIT/GC drift cannot favor one
  // size throughout the comparison. Batching keeps timer granularity small
  // relative to the work; the median tolerates an occasional GC-heavy batch.
  return median(Array.from({ length: PASSES }, (_, pass) => {
    let small: number;
    let large: number;
    if (pass % 2 === 0) {
      small = sampleMs(smallWork);
      large = sampleMs(largeWork);
    } else {
      large = sampleMs(largeWork);
      small = sampleMs(smallWork);
    }
    return large / Math.max(small, 0.5);
  }));
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
    // Ten times the steps must not cost a hundred times the work. The floor
    // matters as much as the ratio: a hundred steps is under a millisecond
    // here, and dividing by a number that small amplifies measurement noise.
    // Thirty leaves room for allocation and GC overhead on top of linear
    // growth, and still catches a per-item scan of
    // the whole list — which is what a return to the old grouping looks like.
    const ratio = growthRatio(flushWork(100), flushWork(1000));
    expect(ratio, `1000 steps cost ${ratio.toFixed(1)}x the CPU time that 100 did`).toBeLessThan(30);
  });
});
