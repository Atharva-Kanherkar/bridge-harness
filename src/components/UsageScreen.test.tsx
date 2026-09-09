// @vitest-environment jsdom
// The usage screen's contract (testing/feat-usage-frontend.md): render the
// summary from the api, toggle the metric without refetching, refetch on a
// window change with the new params, state provenance and coverage plainly,
// and let a history scan refetch the summary.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { bridgeApi } from "../api";
import type { ScanHistoryResult, SummaryParams, UsageBucket, UsageHistorySource, UsageSummaryResult } from "../types";
import { USAGE_PREFERENCES_KEY } from "../usageReport";
import { UsageScreen } from "./UsageScreen";

let container: HTMLDivElement;
let root: Root;
const flush = async () => { await act(async () => {}); };
const text = () => container.textContent ?? "";
const buttonByText = (needle: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === needle)!;
const click = (element: Element) => { act(() => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); }); };

function bucket(day: string, harness: string, model: string, tokens: number, cost: number, extra: Partial<UsageBucket> = {}): UsageBucket {
  return { day, hourStart: null, harness, model, records: 2, sessions: 1, costSource: "model_priced", costMicrousd: cost, cacheSavingsMicrousd: 0, unpricedRecords: 0, totals: { uncachedInputTokens: tokens, cacheReadTokens: 0, cacheWriteTokens: 0, outputTokens: 0, reasoningTokens: 0 }, ...extra };
}

function summaryFor(params: SummaryParams): UsageSummaryResult {
  return {
    buckets: [
      bucket(params.untilDay, "claude", "fable", 1_000_000, 4_500_000),
      bucket(params.untilDay, "codex", "gpt-5.6", 500_000, 0, { costSource: "unpriced", unpricedRecords: 2 }),
    ],
    resolution: params.resolution, sinceDay: params.sinceDay, untilDay: params.untilDay, timeZone: params.timeZone ?? "UTC",
    liveRecords: 4, importedRecords: 0, duplicatesDropped: 3, scanDurationMs: 1,
    pricing: { status: "bundled", source: "litellm", snapshotDate: "2026-08-30", fetchedAt: null, knownModels: 10, overrides: 0 },
    sources: [{ id: "codex:src", agent: "codex", provider: "openai", coverageState: "partial", coverageReason: "scan hit the cap", lastSuccessfulScanAt: null, recordsImported: 5, recordsSkipped: 0 }],
  };
}

const source: UsageHistorySource = { id: "codex:src", agent: "codex", provider: "openai", capability: "supported", location: "~/.codex/sessions", detectedVersion: null, coverageState: "partial", coverageReason: "scan hit the cap", coverageStartAt: null, coverageEndAt: null, lastSuccessfulScanAt: null, lastError: null, recordsImported: 5, recordsSkipped: 0 };

let summarySpy: MockInstance<typeof bridgeApi.usageSummary>;
let scanSpy: MockInstance<typeof bridgeApi.scanUsageHistory>;
let stored: Map<string, string>;

function batch(coverage: "partial" | "complete", recordsImported: number, nextCursor: string): ScanHistoryResult {
  return { durationMs: 1, recordsImported, recordsSkipped: 0, sources: [{ sourceId: source.id, agent: source.agent, provider: source.provider, location: source.location, capability: "supported", coverage, recordsImported, recordsSkipped: 0, nextCursor }] };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom under vitest has no storage on the opaque origin; give the screen a minimal one.
  stored = new Map();
  vi.stubGlobal("localStorage", { getItem: (key: string) => stored.get(key) ?? null, setItem: (key: string, value: string) => { stored.set(key, value); }, removeItem: (key: string) => { stored.delete(key); } });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  summarySpy = vi.spyOn(bridgeApi, "usageSummary").mockImplementation(async params => summaryFor(params));
  scanSpy = vi.spyOn(bridgeApi, "scanUsageHistory").mockResolvedValue({ durationMs: 1, recordsImported: 0, recordsSkipped: 0, sources: [] });
  vi.spyOn(bridgeApi, "listUsageHistorySources").mockResolvedValue([source]);
  vi.spyOn(bridgeApi, "listUsagePriceOverrides").mockResolvedValue([]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function mount() {
  act(() => { root.render(<UsageScreen onError={() => {}} />); });
  await flush();
  await flush();
}

describe("UsageScreen", () => {
  it("renders the cost summary with provenance, coverage, and de-duplication notes", async () => {
    await mount();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(summarySpy.mock.calls[0][0]).toMatchObject({ resolution: "day", includeImported: true });
    expect(text()).toContain("$4.50");
    expect(text()).toContain("API estimate");
    expect(text()).toContain("Partly unpriced");
    expect(text()).toContain("2 records have no known rate");
    expect(text()).toContain("3 live records were counted once");
    expect(text()).toContain("Codex history is still loading.");
    expect(text()).not.toContain("Codex history is partial: scan hit the cap");
    const details = container.querySelector<HTMLDetailsElement>('[aria-label="History sources"] details');
    expect(details?.open).toBe(false);
    expect(details?.textContent).toContain("~/.codex/sessions");
    expect(container.querySelector('svg[role="img"]')?.getAttribute("aria-label")).toBe("Daily cost by harness");
  });

  it("switches the metric without refetching and persists the choice", async () => {
    await mount();
    click(buttonByText("Tokens"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(scanSpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("1.5M");
    expect(container.querySelector('svg[role="img"]')?.getAttribute("aria-label")).toBe("Daily processed tokens by harness");
    expect(JSON.parse(stored.get(USAGE_PREFERENCES_KEY)!)).toMatchObject({ metric: "tokens" });
  });

  it("refetches at hour resolution for the 24h window", async () => {
    await mount();
    click(buttonByText("24h"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(3);
    expect(summarySpy.mock.calls[2][0]).toMatchObject({ resolution: "hour" });
    expect(summarySpy.mock.calls[2][0].sinceTime).toBeTruthy();
    expect(scanSpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("Hourly cost");
  });

  it("scans history and then refetches the summary", async () => {
    scanSpy.mockResolvedValue({ durationMs: 500, recordsImported: 42, recordsSkipped: 0, sources: [] });
    await mount();
    click(buttonByText("Scan history"));
    await flush();
    await flush();
    expect(scanSpy).toHaveBeenCalledTimes(2);
    expect(summarySpy).toHaveBeenCalledTimes(4);
    expect(text()).toContain("Imported 42 records");
  });

  it("says when a window has no activity", async () => {
    summarySpy.mockImplementation(async params => ({ ...summaryFor(params), buckets: [], duplicatesDropped: 0, sources: [] }));
    await mount();
    expect(text()).toContain("No activity in this window.");
    expect(text()).toContain("$0.00");
  });

  it("saves a price override in micro-USD per million tokens", async () => {
    const set = vi.spyOn(bridgeApi, "setUsagePriceOverride").mockResolvedValue([{ model: "fable", inputMicrousdPerMtok: 3_000_000, outputMicrousdPerMtok: 15_000_000, cacheReadMicrousdPerMtok: null, cacheWriteMicrousdPerMtok: null, updatedAt: "2026-09-09T00:00:00Z" }]);
    await mount();
    const editButtons = [...container.querySelectorAll<HTMLButtonElement>("button")].filter(button => button.textContent === "Edit");
    click(editButtons[0]);
    const setValue = (label: string, value: string) => {
      const input = container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      act(() => { setter.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
    };
    setValue("fable input price", "3");
    setValue("fable output price", "15");
    click(buttonByText("Save"));
    await flush();
    expect(set).toHaveBeenCalledWith({ model: "fable", inputMicrousdPerMtok: 3_000_000, outputMicrousdPerMtok: 15_000_000, cacheReadMicrousdPerMtok: null, cacheWriteMicrousdPerMtok: null });
    expect(summarySpy).toHaveBeenCalledTimes(3);
    expect(scanSpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("override");
  });

  it("automatically loads history through every bounded batch and retries only partial sources", async () => {
    const first = batch("partial", 10_000, "cursor-1");
    first.sources.push({ ...first.sources[0], sourceId: "claude:src", agent: "claude", coverage: "complete" });
    scanSpy.mockResolvedValueOnce(first).mockResolvedValueOnce(batch("complete", 25, "cursor-2"));
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([{ ...source, coverageState: "complete" }]);
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(2);
    expect(scanSpy.mock.calls[1][0]).toEqual({ sourceIds: [source.id] });
    expect(text()).toContain("Imported 10,025 records");
    expect(text()).not.toContain("Partial total");
    expect(summarySpy).toHaveBeenCalledTimes(2);
  });

  it("marks the displayed number partial while history is still loading", async () => {
    const pending = deferred<ScanHistoryResult>();
    scanSpy.mockReturnValueOnce(pending.promise);
    await mount();
    expect(text()).toContain("Partial total");
    expect(text()).toContain("Loading history");
    expect(text()).toContain("$4.50");
    await act(async () => { pending.resolve(batch("complete", 5, "end")); });
  });

  it("keeps scanning when the cursor advances even if a batch imports only duplicates", async () => {
    scanSpy.mockResolvedValueOnce(batch("partial", 0, "a"))
      .mockResolvedValueOnce(batch("partial", 0, "b"))
      .mockResolvedValueOnce(batch("complete", 7, "c"));
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(3);
    expect(text()).toContain("Imported 7 records");
  });

  it("stops a stuck cursor and keeps the total labeled partial", async () => {
    scanSpy.mockResolvedValue(batch("partial", 0, "stuck"));
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(2);
    expect(text()).toContain("History scan did not advance");
    expect(text()).toContain("Partial total");
  });

  it("caps auto-scan passes and resumes on demand", async () => {
    let cursors = 0;
    scanSpy.mockImplementation(async () => batch("partial", 0, `cursor-${++cursors}`));
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(25);
    expect(text()).toContain("stopped after 25 batches");
    expect(text()).toContain("Partial total");
  });

  it("does not claim complete coverage when scanning fails", async () => {
    scanSpy.mockRejectedValue(new Error("cannot read history"));
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("cannot read history");
    expect(text()).toContain("Partial total");
  });

  it("continues healthy sources when another source fails", async () => {
    const first = batch("partial", 10_000, "more");
    first.sources.push({ ...first.sources[0], sourceId: "claude:src", agent: "claude", coverage: "unreadable", warning: "Claude history is unreadable" });
    scanSpy.mockResolvedValueOnce(first).mockResolvedValueOnce(batch("complete", 2, "end"));
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([{ ...source, coverageState: "complete" }]);
    await mount();
    expect(scanSpy).toHaveBeenCalledTimes(2);
    expect(scanSpy.mock.calls[1][0]).toEqual({ sourceIds: [source.id] });
    expect(text()).toContain("Claude history is unreadable");
    expect(text()).toContain("Partial total");
  });

  it("keeps the subtotal partial until the post-import summary arrives", async () => {
    const pending = deferred<UsageSummaryResult>();
    summarySpy.mockImplementationOnce(async params => summaryFor(params)).mockReturnValueOnce(pending.promise);
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([{ ...source, coverageState: "complete" }]);
    await mount();
    expect(text()).toContain("$4.50");
    expect(text()).toContain("Partial total");
    await act(async () => { pending.resolve(summaryFor(summarySpy.mock.calls[1][0])); });
    expect(text()).not.toContain("Partial total");
  });

  it("does not leave an old subtotal visible when the post-import summary fails", async () => {
    summarySpy.mockImplementationOnce(async params => summaryFor(params)).mockRejectedValueOnce(new Error("summary unavailable"));
    await mount();
    expect(text()).not.toContain("$4.50");
  });

  it("stops scheduling batches after leaving Usage", async () => {
    const pending = deferred<ScanHistoryResult>();
    scanSpy.mockReturnValueOnce(pending.promise);
    await mount();
    act(() => { root.render(null); });
    await act(async () => { pending.resolve(batch("partial", 10_000, "more")); });
    expect(scanSpy).toHaveBeenCalledTimes(1);
  });

  it("keeps storage paths and scan internals behind an explicit disclosure", async () => {
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([{ ...source, location: "chats/store.db", coverageReason: "ai-tracking/ai-code-tracking.db records have no token counts" }]);
    await mount();
    expect(text()).toContain("Codex history is still loading.");
    expect(text()).not.toContain("Codex history is partial");
    const details = container.querySelector<HTMLDetailsElement>("details");
    expect(details?.open).toBe(false);
    expect(details?.textContent).toContain("chats/store.db");
    expect(details?.textContent).toContain("ai-tracking/ai-code-tracking.db");
  });

  it("ignores an old summary response after the window changes", async () => {
    stored.set(USAGE_PREFERENCES_KEY, JSON.stringify({ metric: "cost", windowDays: 30, includeImported: false }));
    const pending = deferred<UsageSummaryResult>();
    summarySpy.mockReturnValueOnce(pending.promise);
    await mount();
    click(buttonByText("24h"));
    await flush();
    expect(text()).toContain("$4.50");
    const old = summaryFor(summarySpy.mock.calls[0][0]);
    old.buckets[0].costMicrousd = 999_000_000;
    await act(async () => { pending.resolve(old); });
    expect(text()).not.toContain("$999.00");
    expect(text()).toContain("$4.50");
  });
});
