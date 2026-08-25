// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import {
  breakdownMath,
  deltaSummary,
  formatCompactTokens,
  formatTokens,
  rankSegments,
  segmentClassMeta,
  useContextBreakdown,
  type BreakdownFetchers,
} from "./contextBreakdown";
import type { ContextBreakdownResult, ContextBreakdownSegment } from "./protocol/generated/protocol";

function segment(overrides: Partial<ContextBreakdownSegment>): ContextBreakdownSegment {
  return { origin: "adapterInventory", segmentClass: "toolSchemas", state: "measured", capped: false, ...overrides };
}

function result(overrides: Partial<ContextBreakdownResult> = {}): ContextBreakdownResult {
  return {
    sessionId: "session-a",
    digest: "digest-a",
    segments: [],
    totals: { tokens: 900, unavailableSources: 0 },
    conversation: { contextPressure: 0.4, contextWindowTokens: 128_000, entryCount: 4, renderedEntryCount: 4, tokenEstimate: 51_200 },
    ...overrides,
  };
}

describe("rankSegments", () => {
  it("ranks available segments by size then unavailable last, stable within ties", () => {
    const ranked = rankSegments([
      segment({ segmentClass: "skillsPlugins", state: "unavailable", reason: "not exposed" }),
      segment({ segmentClass: "toolSchemas", tokens: 5_000 }),
      segment({ segmentClass: "conversation", origin: "conversation", tokens: 9_000 }),
      segment({ segmentClass: "mcpDynamicTools", tokens: 5_000 }),
      segment({ segmentClass: "agentDefinitions", state: "estimated" }),
      segment({ segmentClass: "providerBaseInstructions", state: "unavailable", reason: "hidden" }),
    ]);
    expect(ranked.map(entry => entry.segment.segmentClass)).toEqual([
      "conversation", "toolSchemas", "mcpDynamicTools", "agentDefinitions", "skillsPlugins", "providerBaseInstructions",
    ]);
  });
});

describe("breakdownMath", () => {
  it("derives known, unattributed, and free regions from the wire payload", () => {
    const math = breakdownMath(result({
      conversation: { contextPressure: 0.78, contextWindowTokens: 200_000, entryCount: 10, renderedEntryCount: 10, tokenEstimate: 156_000 },
      segments: [
        segment({ segmentClass: "conversation", origin: "conversation", tokens: 58_400 }),
        segment({ segmentClass: "toolSchemas", tokens: 21_300 }),
      ],
    }));
    expect(math).toEqual({ knownTokens: 79_700, occupiedTokens: 156_000, windowTokens: 200_000, unattributedTokens: 76_300, freeTokens: 44_000 });
  });

  it("never reports a negative unattributed remainder", () => {
    const math = breakdownMath(result({
      conversation: { contextPressure: 0.2, contextWindowTokens: 128_000, entryCount: 1, renderedEntryCount: 1, tokenEstimate: 3_000 },
      segments: [segment({ segmentClass: "conversation", origin: "conversation", tokens: 9_000 })],
    }));
    expect(math.unattributedTokens).toBe(0);
    expect(math.freeTokens).toBe(125_000);
  });
});

describe("token formatting", () => {
  it("groups thousands and compacts large values", () => {
    expect(formatTokens(58_400)).toBe("58,400");
    expect(formatCompactTokens(58_400)).toBe("58.4k");
    expect(formatCompactTokens(156_000)).toBe("156k");
    expect(formatCompactTokens(8_900)).toBe("8,900");
  });
});

describe("deltaSummary", () => {
  it("signs growth and carries reason and source agent", () => {
    const summary = deltaSummary(result({
      compactionDelta: { boundaryEntryId: "e9", currentTokenEstimate: 156_000, firstRetainedEntryId: "e4", growthTokens: 18_240, reason: "post-checkpoint growth", sourceAgent: "orchestrator", tokensBefore: 137_760 },
    }));
    expect(summary?.text).toBe("+18,240 tok");
    expect(summary?.detail).toBe("post-checkpoint growth · orchestrator");
  });

  it("returns nothing when no compaction snapshot exists", () => {
    expect(deltaSummary(result())).toBeNull();
  });
});

describe("segmentClassMeta", () => {
  it("labels every class bridge-core emits and falls back for unknown ids", () => {
    for (const segmentClass of ["conversation", "prompt-stable", "prompt-variable", "providerBaseInstructions", "toolSchemas", "mcpDynamicTools", "skillsPlugins", "agentDefinitions"]) {
      expect(segmentClassMeta(segmentClass).label).not.toBe(segmentClass);
    }
    expect(segmentClassMeta("prompt-stable").editable).toBe(true);
    expect(segmentClassMeta("prompt-variable").editable).toBe(true);
    expect(segmentClassMeta("futureThing")).toEqual({ label: "Future Thing", group: "adapter" });
  });
});

describe("useContextBreakdown", () => {
  function manualScheduler() {
    const jobs: Array<() => void> = [];
    return {
      jobs,
      scheduler: {
        schedule: (callback: () => void) => { jobs.push(callback); return jobs.length as unknown as ReturnType<typeof setTimeout>; },
        cancel: () => undefined,
      },
    };
  }

  async function flushTurns(turns = 8) {
    await act(async () => {
      for (let index = 0; index < turns; index += 1) await Promise.resolve();
    });
  }

  async function tick(scheduler: ReturnType<typeof manualScheduler>) {
    await act(async () => {
      const job = scheduler.jobs.shift();
      job?.();
      for (let index = 0; index < 8; index += 1) await Promise.resolve();
    });
  }

  async function mountHook(
    sessionId: string | null,
    enabled: boolean,
    fetchers: BreakdownFetchers,
    poller = manualScheduler(),
  ) {
    let latest!: ReturnType<typeof useContextBreakdown>;
    function Harness() {
      latest = useContextBreakdown(sessionId, enabled, fetchers, 15_000, poller.scheduler);
      return null;
    }
    const container = document.createElement("div");
    document.body.append(container);
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
    });
    await flushTurns();
    return {
      get: () => latest,
      poller,
      unmount: async () => act(async () => { root.unmount(); }),
    };
  }

  function trackingFetchers(digestValue: string | Error, resultValue?: ContextBreakdownResult): BreakdownFetchers & { digestCalls: number; fetchCalls: number } {
    const calls = { digestCalls: 0, fetchCalls: 0 };
    return {
      get digestCalls() { return calls.digestCalls; },
      get fetchCalls() { return calls.fetchCalls; },
      digest: () => { calls.digestCalls += 1; return digestValue instanceof Error ? Promise.reject(digestValue) : Promise.resolve(digestValue); },
      fetch: () => { calls.fetchCalls += 1; return Promise.resolve(resultValue ?? result()); },
    };
  }

  it("fetches only when the digest token changes", async () => {
    let token = "d1";
    const fetchers: BreakdownFetchers & { digestCalls: number; fetchCalls: number } = {
      digestCalls: 0,
      fetchCalls: 0,
      digest: () => { fetchers.digestCalls += 1; return Promise.resolve(token); },
      fetch: () => { fetchers.fetchCalls += 1; return Promise.resolve(result({ digest: token })); },
    };
    const handle = await mountHook("session-a", true, fetchers);
    expect(fetchers.digestCalls).toBe(1);
    expect(fetchers.fetchCalls).toBe(1);
    expect(handle.get().result?.sessionId).toBe("session-a");
    expect(handle.poller.jobs.length).toBe(1);

    await tick(handle.poller);
    expect(fetchers.digestCalls).toBe(2);
    expect(fetchers.fetchCalls).toBe(1);

    token = "d2";
    await tick(handle.poller);
    expect(fetchers.fetchCalls).toBe(2);
    expect(handle.get().reconciledAt).not.toBeNull();
    await handle.unmount();
  });

  it("discards prior session state instead of mixing digests", async () => {
    const seen: string[] = [];
    const fetchers: BreakdownFetchers & { digestCalls: number } = {
      digestCalls: 0,
      digest: id => { seen.push(id); fetchers.digestCalls += 1; return Promise.resolve(`d-${id}`); },
      fetch: id => Promise.resolve(result({ sessionId: id, digest: `d-${id}` })),
    };
    const container = document.createElement("div");
    document.body.append(container);
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const root = createRoot(container);
    let latest!: ReturnType<typeof useContextBreakdown>;
    function Harness({ sessionId }: { sessionId: string }) {
      latest = useContextBreakdown(sessionId, true, fetchers, 15_000);
      return null;
    }
    await act(async () => { root.render(<Harness sessionId="a" />); });
    await flushTurns();
    expect(latest.result?.sessionId).toBe("a");
    await act(async () => { root.render(<Harness sessionId="b" />); });
    await flushTurns();
    expect(latest.result?.sessionId).toBe("b");
    expect(seen.filter(id => id === "a").length).toBe(1);
    await flushTurns();
    expect(seen.filter(id => id === "a").length).toBe(1);
    await act(async () => { root.unmount(); });
  });

  it("stops cleanly when the backend method is missing", async () => {
    const fetchers = trackingFetchers(new Error("Command sessions/get_context_breakdown_digest not found"));
    const handle = await mountHook("session-a", true, fetchers);
    expect(fetchers.digestCalls).toBe(1);
    expect(handle.get().unavailable).toBe(true);
    expect(handle.get().result).toBeNull();
    while (handle.poller.jobs.length) await tick(handle.poller);
    expect(fetchers.digestCalls).toBe(1);
    await handle.unmount();
  });

  it("gives up after repeated transient failures without surfacing errors", async () => {
    const fetchers = trackingFetchers(new Error("bridge core unavailable"));
    const handle = await mountHook("session-a", true, fetchers);
    expect(fetchers.digestCalls).toBe(1);
    expect(handle.get().unavailable).toBe(false);
    await tick(handle.poller);
    expect(fetchers.digestCalls).toBe(2);
    expect(handle.get().unavailable).toBe(false);
    await tick(handle.poller);
    expect(fetchers.digestCalls).toBe(3);
    expect(handle.get().unavailable).toBe(true);
    expect(handle.get().result).toBeNull();
    expect(handle.poller.jobs.length).toBe(0);
    await handle.unmount();
  });

  it("does not poll while the panel is closed or the session is unknown", async () => {
    const fetchers = trackingFetchers("d1");
    const closed = await mountHook("session-a", false, fetchers);
    expect(fetchers.digestCalls).toBe(0);
    expect(closed.get().result).toBeNull();
    await closed.unmount();

    const sessionless = await mountHook(null, true, fetchers);
    expect(fetchers.digestCalls).toBe(0);
    expect(sessionless.get().result).toBeNull();
    await sessionless.unmount();
  });
});
