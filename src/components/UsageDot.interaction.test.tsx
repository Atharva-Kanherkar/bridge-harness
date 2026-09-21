// @vitest-environment jsdom
// The live dot: subscribes to the shared provider overviews, opens and closes
// its card, refreshes through the same interactive method the menu bar uses,
// and portals onto the composer frame so the card hangs from the composer.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ProviderUsageOverviews } from "../protocol/generated/protocol";
import { ChatUsageDot } from "./UsageDot";

let container: HTMLDivElement;
let root: Root;
let listener: ((value: ProviderUsageOverviews) => void) | undefined;

const empty = { tokens: { status: "unavailable" as const }, costMicrousd: { status: "unavailable" as const }, models: [] };

function overviews(weekly: number, generatedAt = Math.floor(Date.now() / 1000)): ProviderUsageOverviews {
  // `generatedAt` orders pushes; the observation itself must never sit in the
  // future or the dot rightly treats it as not yet live.
  const observedAt = Math.min(generatedAt, Math.floor(Date.now() / 1000));
  return {
    schemaVersion: 1, generatedAt,
    providers: [{ schemaVersion: 1, generatedAt, provider: "codex", observedAt, coverage: "test", today: empty, month: empty, error: null,
      windows: [
        { id: "session", label: "5-hour", usedPercent: { value: 0, source: "reported", status: "current" }, resetsAt: generatedAt + 3600, windowMinutes: 300 },
        { id: "weekly", label: "Weekly", usedPercent: { value: weekly, source: "reported", status: "current" }, resetsAt: generatedAt + 86400, windowMinutes: 10080 },
      ] }],
  };
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  container.setAttribute("data-composer-frame", "");
  document.body.append(container);
  root = createRoot(container);
  listener = undefined;
  vi.spyOn(bridgeApi, "getProviderUsageOverviews").mockResolvedValue(overviews(63));
  vi.spyOn(bridgeApi, "onProviderUsageOverviews").mockImplementation(async handler => { listener = handler; return () => { listener = undefined; }; });
  vi.spyOn(bridgeApi, "refreshProviderUsageOverviews").mockResolvedValue(overviews(71, Math.floor(Date.now() / 1000) + 5));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function mount(props: Partial<React.ComponentProps<typeof ChatUsageDot>> = {}) {
  await act(async () => { root.render(<ChatUsageDot {...props} />); });
  await act(async () => { await Promise.resolve(); });
}

const trigger = () => container.querySelector<HTMLButtonElement>('button[aria-controls="usage-dot-panel"]')!;
const panel = () => container.querySelector<HTMLElement>("#usage-dot-panel")!;

describe("ChatUsageDot", () => {
  it("loads the shared overviews and headlines the tightest window", async () => {
    await mount();
    expect(bridgeApi.getProviderUsageOverviews).toHaveBeenCalledTimes(1);
    expect(trigger().getAttribute("aria-label")).toBe("Open usage — healthy, 63% of Codex Weekly used");
  });

  it("opens on click, closes on Escape, and portals onto the composer frame", async () => {
    await mount();
    expect(panel().className).toContain("invisible");
    expect(panel().parentElement).toBe(container);
    await act(async () => { trigger().click(); });
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
    expect(panel().className).toContain("opacity-100");
    expect(panel().textContent).toContain("Weekly");
    expect(panel().textContent).toContain("63% used");
    await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); });
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
  });

  it("follows pushed overviews and refreshes through the interactive method", async () => {
    await mount();
    await act(async () => { listener?.(overviews(88, Math.floor(Date.now() / 1000) + 1)); });
    expect(trigger().getAttribute("aria-label")).toContain("elevated, 88%");
    await act(async () => { trigger().click(); });
    const refresh = panel().querySelector<HTMLButtonElement>('button[aria-label="Refresh usage"]')!;
    await act(async () => { refresh.click(); });
    await act(async () => { await Promise.resolve(); });
    expect(bridgeApi.refreshProviderUsageOverviews).toHaveBeenCalledTimes(1);
    // The refresh result is the newest snapshot, so it replaces the push.
    expect(trigger().getAttribute("aria-label")).toContain("71%");
  });

  it("keeps a refreshed snapshot when an older event arrives late", async () => {
    await mount();
    const base = Math.floor(Date.now() / 1000);
    vi.mocked(bridgeApi.refreshProviderUsageOverviews).mockResolvedValue(overviews(71, base + 5));
    await act(async () => { trigger().click(); });
    await act(async () => { panel().querySelector<HTMLButtonElement>('button[aria-label="Refresh usage"]')!.click(); });
    await act(async () => { await Promise.resolve(); });
    expect(trigger().getAttribute("aria-label")).toContain("71%");
    await act(async () => { listener?.(overviews(20, base + 4)); });
    expect(trigger().getAttribute("aria-label")).toContain("71%");
  });

  it("accepts a current pushed reading between the meter clock's 30-second ticks", async () => {
    const wallClock = vi.spyOn(Date, "now").mockReturnValue(1_800_000_000_000);
    await mount();
    wallClock.mockReturnValue(1_800_000_002_000);
    await act(async () => { listener?.(overviews(88)); });
    expect(trigger().getAttribute("aria-label")).toContain("elevated, 88%");
  });

  it("shows a failed initial load instead of loading forever, and clears it when a snapshot arrives", async () => {
    vi.mocked(bridgeApi.getProviderUsageOverviews).mockRejectedValue(new Error("Usage unavailable"));
    await mount();
    await act(async () => { trigger().click(); });
    expect(panel().textContent).not.toContain("Loading usage");
    expect(panel().querySelector('[role="alert"]')!.textContent).toContain("Usage unavailable");
    await act(async () => { listener?.(overviews(63)); });
    expect(panel().querySelector('[role="alert"]')).toBeNull();
    expect(panel().textContent).toContain("63% used");
  });

  it("reports a failed refresh and keeps the last reading", async () => {
    vi.mocked(bridgeApi.refreshProviderUsageOverviews).mockRejectedValue(new Error("Codex account refresh timed out. Try again."));
    await mount();
    await act(async () => { trigger().click(); });
    await act(async () => { panel().querySelector<HTMLButtonElement>('button[aria-label="Refresh usage"]')!.click(); });
    await act(async () => { await Promise.resolve(); });
    expect(panel().querySelector('[role="alert"]')!.textContent).toContain("timed out");
    expect(panel().textContent).toContain("63% used");
    expect(panel().querySelector<HTMLButtonElement>('button[aria-label="Refresh usage"]')!.disabled).toBe(false);
  });

  it("drops an older push instead of letting it overwrite a newer reading", async () => {
    await mount();
    await act(async () => { listener?.(overviews(20, 1)); });
    expect(trigger().getAttribute("aria-label")).toContain("63%");
  });

  it("routes Open Usage and closes the card", async () => {
    const onOpenUsage = vi.fn();
    await mount({ onOpenUsage });
    await act(async () => { trigger().click(); });
    const open = [...panel().querySelectorAll("button")].find(button => button.textContent?.includes("Open Usage"))!;
    await act(async () => { open.click(); });
    expect(onOpenUsage).toHaveBeenCalledTimes(1);
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
  });
});
