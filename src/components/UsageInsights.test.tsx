// @vitest-environment jsdom
// The Insights tab contract: the stored report loads without running a model;
// "Analyse" is the only thing that does; every chart is drawn from Bridge's
// figures with the harness's colour; the states without a report say why.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { UsageInsightsResult } from "../types";
import { UsageInsights } from "./UsageInsights";

let container: HTMLDivElement;
let root: Root;

const READY: UsageInsightsResult = {
  status: "ready",
  windowDays: 30,
  generatedAt: new Date().toISOString(),
  harness: "claude",
  model: "sonnet",
  report: {
    headline: "Afternoons on Claude",
    summary: "Most prompting lands after lunch.",
    highlights: [{ title: "Cache is doing the work", detail: "Two thirds cached.", tone: "good" }],
    themes: [{ label: "Refactors", share: 0.6, example: "Tidy the meter" }, { label: "Bugs", share: 0.4 }],
    recommendations: ["Route reviews to Codex."],
    harnesses: [
      { harness: "claude", processedTokens: 4_000_000, costMicrousd: 3_000_000, records: 40, sessions: 4, prompts: 20 },
      { harness: "codex", processedTokens: 1_000_000, costMicrousd: 500_000, records: 10, sessions: 2, prompts: 6 },
    ],
    hours: Array.from({ length: 24 }, (_, hour) => ({ hour, prompts: hour === 15 ? 9 : hour === 10 ? 3 : 0 })),
    days: [{ day: "2026-09-08", processedTokens: 2_000_000, prompts: 10 }, { day: "2026-09-09", processedTokens: 3_000_000, prompts: 16 }],
    github: { repositories: 2, openPrs: 5, draftPrs: 1, failingChecks: 2, awaitingReview: 1 },
    promptsAnalysed: 26,
  },
};

const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("UsageInsights", () => {
  it("shows the stored report without asking the model, and analyses only on click", async () => {
    const spy = vi.spyOn(bridgeApi, "usageInsights").mockResolvedValue(READY);
    await act(async () => { root.render(<UsageInsights windowDays={30} onError={() => undefined} />); });
    await settle();
    expect(spy).toHaveBeenCalledWith({ windowDays: 30, refresh: false });
    const text = container.textContent ?? "";
    expect(text).toContain("Afternoons on Claude");
    expect(text).toContain("Written by Claude");
    expect(text).toContain("Cache is doing the work");
    expect(text).toContain("Refactors");
    expect(text).toContain("60%");
    expect(text).toContain("Route reviews to Codex.");
    expect(text).toContain("Busiest at 15:00");
    // GitHub tiles from Bridge's own figures.
    expect(container.querySelector('[aria-label="GitHub and recommendations"]')?.textContent).toContain("Failing checks");
    // Series colour follows the harness.
    expect(container.querySelector('[aria-label="Processed tokens by harness"] .bg-chart-claude')).not.toBeNull();
    expect(container.querySelector('[aria-label="Processed tokens by harness"] .bg-chart-codex')).not.toBeNull();
    // Nothing yellow.
    expect(container.innerHTML).not.toMatch(/warning/);

    const button = [...container.querySelectorAll("button")].find(item => item.textContent?.includes("Analyse again"))!;
    await act(async () => { button.click(); });
    expect(spy).toHaveBeenLastCalledWith({ windowDays: 30, refresh: true });
  });

  it("reads the harness bars and the day chart on hover", async () => {
    vi.spyOn(bridgeApi, "usageInsights").mockResolvedValue(READY);
    await act(async () => { root.render(<UsageInsights windowDays={30} onError={() => undefined} />); });
    await settle();
    const claude = container.querySelector<HTMLElement>('[aria-label="Processed tokens by harness"] li')!;
    act(() => { claude.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(container.querySelector('[role="tooltip"]')?.textContent).toContain("40 requests");
    act(() => { claude.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })); });
    const plot = container.querySelector<HTMLElement>('[aria-label="Processed tokens by day"] div')!;
    plot.getBoundingClientRect = () => ({ left: 0, width: 200, top: 0, height: 128, right: 200, bottom: 128, x: 0, y: 0, toJSON: () => ({}) });
    act(() => { plot.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 190 })); });
    expect(container.querySelector('[role="tooltip"]')?.textContent).toContain("2026-09-09");
  });

  it("explains an empty, failed, or unavailable state instead of an empty page", async () => {
    vi.spyOn(bridgeApi, "usageInsights").mockResolvedValue({ status: "unavailable", windowDays: 30, detail: "Insights need Claude Code installed." });
    await act(async () => { root.render(<UsageInsights windowDays={30} onError={() => undefined} />); });
    await settle();
    expect(container.textContent).toContain("Insights need a harness");
    expect(container.textContent).toContain("Insights need Claude Code installed.");
    // Nothing to click when no harness can run it.
    expect([...container.querySelectorAll("button")].some(item => item.textContent?.includes("Analyse"))).toBe(false);
  });

  it("surfaces a transport failure through onError and keeps the retry", async () => {
    vi.spyOn(bridgeApi, "usageInsights").mockRejectedValue(new Error("daemon away"));
    const onError = vi.fn();
    await act(async () => { root.render(<UsageInsights windowDays={7} onError={onError} />); });
    await settle();
    expect(onError).toHaveBeenCalledWith("daemon away");
    expect(container.textContent).toContain("The analysis did not finish");
    expect([...container.querySelectorAll("button")].some(item => item.textContent?.includes("Analyse my usage"))).toBe(true);
  });
});
