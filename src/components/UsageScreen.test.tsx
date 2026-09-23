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
import { formatDayShort, USAGE_LAYOUT_OPTIONS, USAGE_PREFERENCES_KEY, type UsageLayout } from "../usageReport";
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

/** Pin a layout (and optionally a metric) before mounting, the way a returning user would have it stored. */
function prefer(layout: UsageLayout | string, metric: "cost" | "tokens" = "cost") {
  stored.set(USAGE_PREFERENCES_KEY, JSON.stringify({ metric, windowDays: 30, includeImported: true, layout }));
}

async function mount() {
  act(() => { root.render(<UsageScreen onError={() => {}} />); });
  await flush();
  await flush();
}

describe("UsageScreen", () => {
  it("renders the cost summary with provenance, coverage, and de-duplication notes", async () => {
    prefer("classic");
    await mount();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(summarySpy.mock.calls[0][0]).toMatchObject({ resolution: "day", includeImported: true, includeDashboard: true });
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
    prefer("classic");
    await mount();
    click(buttonByText("Tokens"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(scanSpy).toHaveBeenCalledTimes(1);
    expect(text()).toContain("1.5M");
    expect(container.querySelector('svg[role="img"]')?.getAttribute("aria-label")).toBe("Daily processed tokens by harness");
    expect(JSON.parse(stored.get(USAGE_PREFERENCES_KEY)!)).toMatchObject({ metric: "tokens" });
  });

  it("includes dashboard totals and shows Cursor as cached account history", async () => {
    const cursor: UsageHistorySource = {
      ...source, id: "cursor:dashboard", agent: "cursor", provider: "cursor", origin: "dashboard",
      location: "https://cursor.com/dashboard", coverageState: "complete", recordsImported: 25,
      coverageStartAt: "2026-08-15T00:00:00Z", coverageEndAt: "2026-09-13T12:00:00Z",
      lastSuccessfulScanAt: "2026-09-13T12:00:00Z", coverageReason: "Dashboard history as of the last update.",
    };
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([cursor]);
    summarySpy.mockImplementation(async params => ({
      ...summaryFor(params), duplicatesDropped: 0,
      buckets: [bucket(params.untilDay, "cursor", "cursor-model", 2_000_000, 12_500_000, { sessions: null, costSource: "provider_reported" })],
      sources: [cursor],
    }));
    await mount();
    expect(text()).toContain("$12.50");
    expect(text()).toContain("Dashboard usage · last 30 days");
    expect(text()).toContain("25 cached events");
    expect(text()).not.toContain("Cursor history is still loading");
    expect(text()).not.toContain("Unsupported");
    expect(text()).not.toContain("25 imported");
    expect(text()).not.toContain("Partial total");
    click(buttonByText("Tokens"));
    await flush();
    expect(text()).toContain("2M");
    expect(summarySpy).toHaveBeenCalledTimes(2);
  });

  it("keeps stale dashboard totals visible and explains unsupported hourly coverage", async () => {
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([]);
    summarySpy.mockImplementation(async params => ({
      ...summaryFor(params), duplicatesDropped: 0,
      buckets: params.resolution === "day" ? [bucket(params.untilDay, "cursor", "cursor-model", 10, 12_500_000, { sessions: null })] : [],
      sources: [{ id: "cursor:dashboard", origin: "dashboard", agent: "cursor", provider: "cursor", recordsImported: 1, recordsSkipped: 0, lastSuccessfulScanAt: "2026-09-13T12:00:00Z",
        coverageState: params.resolution === "day" ? "stale" : "unsupported",
        coverageReason: params.resolution === "day" ? "Showing last-known Cursor usage." : "Cursor dashboard history supports daily totals. Choose 7d or longer to include it." }],
    }));
    await mount();
    expect(text()).toContain("$12.50");
    expect(text()).toContain("Showing last-known Cursor usage.");
    expect(text()).toContain("History incomplete");
    click(buttonByText("24h"));
    await flush();
    expect(text()).toContain("Choose 7d or longer");
    expect(text()).not.toContain("$12.50");
    expect(text()).not.toContain("Cursor history is still loading");
  });

  it("labels a known dashboard cost subtotal when some events are unpriced", async () => {
    vi.mocked(bridgeApi.listUsageHistorySources).mockResolvedValue([]);
    summarySpy.mockImplementation(async params => ({
      ...summaryFor(params), duplicatesDropped: 0, sources: [],
      buckets: [bucket(params.untilDay, "cursor", "partly-priced", 10, 12_500_000, { sessions: null, costSource: "unpriced", unpricedRecords: 1 })],
    }));
    await mount();
    expect(text()).toContain("$12.50");
    expect(text()).toContain("Partly unpriced");
    expect(text()).toContain("1 records have no known rate");
    expect(text()).toContain("their cost is unknown, not free");
  });

  it("refetches at hour resolution for the 24h window", async () => {
    prefer("classic");
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

  it("keeps the activity calendar behind a disclosure, closed by default", async () => {
    prefer("classic");
    await mount();
    expect(container.querySelector('[role="grid"]')).toBeNull();
    const toggle = container.querySelector<HTMLButtonElement>('[aria-controls="usage-activity"]')!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector('[role="grid"]')).not.toBeNull();
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

describe("UsageScreen layouts", () => {
  const radio = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Layout"] [role="radio"]')].find(button => button.textContent === label)!;
  const section = (label: string) => container.querySelector<HTMLElement>(`[aria-label="${label}"]`)!;

  it.each(USAGE_LAYOUT_OPTIONS.filter(layout => layout !== "classic"))("keeps every honesty note in the %s layout", async layout => {
    prefer(layout);
    await mount();
    expect(radio(layout[0].toUpperCase() + layout.slice(1)).getAttribute("aria-checked")).toBe("true");
    expect(text()).toContain("$4.50~");
    expect(text()).toContain("API estimate · Partly unpriced");
    expect(text()).toContain("Partial total · History incomplete");
    expect(text()).toContain("2 records have no known rate");
    expect(text()).toContain("3 live records were counted once");
    expect(text()).toContain("Codex history is still loading.");
    expect(section("History sources")).not.toBeNull();
    expect(section("Model prices")).not.toBeNull();
  });

  it("defaults to Ledger, switches layout without refetching, and persists it", async () => {
    await mount();
    expect(radio("Ledger").getAttribute("aria-checked")).toBe("true");
    expect(text()).toContain("Itemised by model");
    click(radio("Strips"));
    await flush();
    expect(summarySpy).toHaveBeenCalledTimes(2);
    expect(section("Strip scale")).not.toBeNull();
    expect(JSON.parse(stored.get(USAGE_PREFERENCES_KEY)!)).toMatchObject({ layout: "strips", metric: "cost", windowDays: 30 });
  });

  it("falls back to Ledger for a stored layout it does not know", async () => {
    prefer("radar", "tokens");
    await mount();
    expect(radio("Ledger").getAttribute("aria-checked")).toBe("true");
    expect(buttonByText("Tokens").getAttribute("aria-checked")).toBe("true");
  });

  it("keeps the breakdown table one disclosure away in the new layouts", async () => {
    await mount();
    const toggle = container.querySelector<HTMLButtonElement>('[aria-controls="usage-breakdown-table"]')!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelector("#usage-breakdown-table")).toBeNull();
    click(toggle);
    expect(container.querySelector("#usage-breakdown-table table")).not.toBeNull();
  });

  it("scrubs every strip at once with the arrow keys", async () => {
    prefer("strips");
    await mount();
    const plot = container.querySelector<HTMLElement>('[aria-label="Daily cost"] [role="group"]')!;
    expect(plot.tabIndex).toBe(0);
    expect(container.querySelector('[aria-live="polite"]')).toBeNull();
    act(() => { plot.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })); });
    const readout = container.querySelector('[aria-live="polite"]')!;
    expect(readout.textContent).toContain("Claude$4.50");
    expect(readout.textContent).toContain("Codex$0.00");
    expect(readout.textContent).toContain("Total$4.50");
    act(() => { plot.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })); });
    expect(container.querySelector('[aria-live="polite"]')!.textContent).toContain("Total$0.00");
  });

  it("stops the flow at the model in cost mode and follows tokens to their kind", async () => {
    prefer("flow");
    await mount();
    expect(text()).toContain("Cost is recorded per model, not per token kind");
    expect(container.querySelector('svg[aria-label="Cost flowing from harness to model"]')).not.toBeNull();
    click(buttonByText("Tokens"));
    await flush();
    expect(text()).not.toContain("Cost is recorded per model");
    expect(container.querySelector('svg[aria-label="Processed tokens flowing from harness to model to token kind"]')?.textContent).toContain("Uncached input");
  });

  it("scopes the calendar headline to the picked days and clears it with the window", async () => {
    prefer("calendar");
    await mount();
    const days = () => [...section("Daily calendar").querySelectorAll<HTMLButtonElement>("button[aria-pressed]")];
    expect(days()).toHaveLength(30);
    expect(section("Selection").textContent).toContain("Whole window");
    expect(section("Selection").textContent).toContain("$4.50");
    click(days()[0]);
    expect(days()[0].getAttribute("aria-pressed")).toBe("true");
    expect(section("Selection").textContent).toContain("Selected");
    expect(section("Selection").textContent).toContain("$0.00");
    expect(section("Selection").textContent).toContain("1 day");
    act(() => { days()[29].dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })); });
    expect(days().every(day => day.getAttribute("aria-pressed") === "true")).toBe(true);
    expect(section("Selection").textContent).toContain("30 days");
    expect(section("Selection").textContent).toContain("$4.50");
    expect(days()[29].getAttribute("aria-label")).toContain(formatDayShort(summarySpy.mock.calls[0][0].untilDay));
    click(buttonByText("7d"));
    await flush();
    expect(section("Selection").textContent).toContain("Whole window");
    expect(days()).toHaveLength(7);
  });
});
