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
    // Planned providers are not a limit and take no room on the card.
    expect(container.textContent).not.toContain("providers planned");
  });

  it("drops a provider whose limits have expired instead of showing the old number", async () => {
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
    expect(container.textContent).toContain("44%");

    // The window resets with no session running, so the disk path reports an
    // empty payload. Ignoring it would leave 44% on screen indefinitely.
    await act(async () => { push?.({ provider: "codex", rateLimits: {} }); });
    await settle();

    expect(container.textContent).not.toContain("44%");
    expect(container.textContent).toContain("no limits");
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

  it("offers a way into the app, dismissing itself first", async () => {
    const hide = vi.spyOn(bridgeApi, "hideMeterPanel").mockResolvedValue();
    const reveal = vi.spyOn(bridgeApi, "revealMainWindow").mockResolvedValue();
    vi.spyOn(bridgeApi, "getMeterSnapshot").mockResolvedValue(REGISTRY as never);
    vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    vi.spyOn(bridgeApi, "onMeterTray").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onAccountUsage").mockResolvedValue(() => undefined);

    await act(async () => { root.render(<MeterPanel />); });
    await settle();
    const open = [...container.querySelectorAll("button")].find(button => button.textContent === "Open Bridge");
    expect(open).toBeDefined();
    await act(async () => { open!.click(); });

    expect(reveal).toHaveBeenCalledTimes(1);
    // The panel must not be left floating over the window it just raised.
    expect(hide).toHaveBeenCalledTimes(1);
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
