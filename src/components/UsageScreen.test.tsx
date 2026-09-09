// @vitest-environment jsdom
// The usage screen's contract (testing/feat-usage-frontend.md): render the
// summary from the api, toggle the metric without refetching, refetch on a
// window change with the new params, state provenance and coverage plainly,
// and let a history scan refetch the summary.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { bridgeApi } from "../api";
import type { SummaryParams, UsageBucket, UsageHistorySource, UsageSummaryResult } from "../types";
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
let stored: Map<string, string>;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom under vitest has no storage on the opaque origin; give the screen a minimal one.
  stored = new Map();
  vi.stubGlobal("localStorage", { getItem: (key: string) => stored.get(key) ?? null, setItem: (key: string, value: string) => { stored.set(key, value); }, removeItem: (key: string) => { stored.delete(key); } });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  summarySpy = vi.spyOn(bridgeApi, "usageSummary").mockImplementation(async params => summaryFor(params));
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
    expect(summarySpy).toHaveBeenCalledTimes(1);
    expect(summarySpy.mock.calls[0][0]).toMatchObject({ resolution: "day", includeImported: true });
    expect(text()).toContain("$4.50");
    expect(text()).toContain("API estimate");
    expect(text()).toContain("Partly unpriced");
    expect(text()).toContain("2 records have no known rate");
    expect(text()).toContain("3 live records were counted once");
    expect(text()).toContain("Codex history is partial: scan hit the cap");
    expect(container.querySelector('[aria-label="History sources"]')?.textContent).toContain("~/.codex/sessions");
    expect(container.querySelector('svg[role="img"]')?.getAttribute("aria-label")).toBe("Daily cost by harness");
  });

  it("switches the metric without refetching and persists the choice", async () => {
    await mount();
    click(buttonByText("Tokens"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("1.5M");
    expect(container.querySelector('svg[role="img"]')?.getAttribute("aria-label")).toBe("Daily processed tokens by harness");
    expect(JSON.parse(stored.get(USAGE_PREFERENCES_KEY)!)).toMatchObject({ metric: "tokens" });
  });

  it("refetches at hour resolution for the 24h window", async () => {
    await mount();
    click(buttonByText("24h"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(summarySpy.mock.calls[1][0]).toMatchObject({ resolution: "hour" });
    expect(summarySpy.mock.calls[1][0].sinceTime).toBeTruthy();
    expect(text()).toContain("Hourly cost");
  });

  it("scans history and then refetches the summary", async () => {
    const scan = vi.spyOn(bridgeApi, "scanUsageHistory").mockResolvedValue({ durationMs: 500, recordsImported: 42, recordsSkipped: 0, sources: [] });
    await mount();
    click(buttonByText("Scan history"));
    await flush();
    await flush();
    expect(scan).toHaveBeenCalledTimes(1);
    expect(summarySpy).toHaveBeenCalledTimes(2);
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
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(text()).toContain("override");
  });
});
