// @vitest-environment jsdom
// The menu-bar panel contract. This surface runs in its own window with no App
// around it, so the thing worth testing is that it feeds itself: registry,
// usage stream, refresh, and a close that hides the window rather than
// unmounting a modal.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { MeterPanel } from "./MeterPanel";

let container: HTMLDivElement;
let root: Root;

const REGISTRY = {
  providers: [
    { id: "codex", label: "Codex", supported: true },
    { id: "gemini", label: "Gemini", supported: false, plannedSource: "CodexBar Gemini probe" },
  ],
  adaptiveDefaultSeconds: 300,
  nominalIntervalSeconds: 300,
  attribution: "CodexBar (MIT)",
};

/** The shape the account-usage channel delivers for Codex. */
const CODEX_FRAME = {
  provider: "codex",
  rateLimits: {
    primary: { used_percent: 44, window_minutes: 300, resets_in_seconds: 900 },
    secondary: { used_percent: 8, window_minutes: 10_080, resets_in_seconds: 500_000 },
    plan_type: "plus",
  },
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function settle(times = 3) {
  for (let index = 0; index < times; index += 1) {
    await act(async () => { await Promise.resolve(); });
  }
}

describe("MeterPanel", () => {
  it("loads its own registry and renders usage pushed on the account channel", async () => {
    let push: ((payload: { provider: string; rateLimits: unknown }) => void) | undefined;
    vi.spyOn(bridgeApi, "getMeterSnapshot").mockResolvedValue(REGISTRY as never);
    vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    vi.spyOn(bridgeApi, "onMeterTray").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onAccountUsage").mockImplementation(async handler => {
      push = handler as never;
      return () => undefined;
    });

    await act(async () => { root.render(<MeterPanel />); });
    await settle();
    await act(async () => { push?.(CODEX_FRAME); });
    await settle();

    expect(container.textContent).toContain("Codex");
    // The 5h window, and the weekly one, both from the pushed frame.
    expect(container.textContent).toContain("44%");
    expect(container.textContent).toContain("8%");
    // The worst window is the headline.
    expect(container.textContent).toContain("44% worst");
    // Planned providers stay visible rather than silently missing.
    expect(container.textContent).toContain("1 more providers planned");
  });

  it("refreshes on mount so an opened panel is never showing stale numbers", async () => {
    const refresh = vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    vi.spyOn(bridgeApi, "getMeterSnapshot").mockResolvedValue(REGISTRY as never);
    vi.spyOn(bridgeApi, "onMeterTray").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onAccountUsage").mockResolvedValue(() => undefined);

    await act(async () => { root.render(<MeterPanel />); });
    await settle();

    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("closes by hiding its window, not by unmounting itself", async () => {
    const hide = vi.spyOn(bridgeApi, "hideMeterPanel").mockResolvedValue();
    vi.spyOn(bridgeApi, "getMeterSnapshot").mockResolvedValue(REGISTRY as never);
    vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    vi.spyOn(bridgeApi, "onMeterTray").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onAccountUsage").mockResolvedValue(() => undefined);

    await act(async () => { root.render(<MeterPanel />); });
    await settle();
    const close = container.querySelector<HTMLButtonElement>('[aria-label="Close meter"]');
    expect(close).not.toBeNull();
    await act(async () => { close!.click(); });

    expect(hide).toHaveBeenCalledTimes(1);
    // Still mounted: the window went away, the React tree did not.
    expect(container.querySelector('[role="dialog"][aria-label="Usage meter"]')).not.toBeNull();
  });

  it("is not a modal — nothing sits behind it to make inert", async () => {
    vi.spyOn(bridgeApi, "getMeterSnapshot").mockResolvedValue(REGISTRY as never);
    vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    vi.spyOn(bridgeApi, "onMeterTray").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onAccountUsage").mockResolvedValue(() => undefined);

    await act(async () => { root.render(<MeterPanel />); });
    await settle();

    const dialog = container.querySelector('[role="dialog"][aria-label="Usage meter"]');
    expect(dialog?.getAttribute("aria-modal")).toBeNull();
  });
});
