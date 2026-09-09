// @vitest-environment jsdom
// The meter popover contract: live provider windows render with usage bars,
// reset countdowns and a ring gauge per provider; nothing that
// is not a live limit (planned providers, attribution) takes up the card;
// refresh and close are wired.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UsageSnapshot } from "../../usage";
import type { MeterRegistry } from "../../types";
import { MeterPopover } from "./MeterPopover";

let container: HTMLDivElement;
let root: Root;

const WEEK_SECONDS = 10_080 * 60;

function snapshot(usedPercent: number, resetsInSeconds: number): UsageSnapshot {
  return {
    windows: [{ id: "weekly", label: "Weekly", usedPercent, windowMinutes: 10_080, resetsInSeconds, source: "reported" }],
    source: "reported",
    capturedAt: new Date().toISOString(),
  };
}

const registry: MeterRegistry = {
  providers: [
    { id: "codex", label: "Codex", supported: true, plannedSource: null },
    { id: "claude", label: "Claude", supported: true, plannedSource: null },
    { id: "openrouter", label: "OpenRouter", supported: false, plannedSource: "API token credit tracking" },
  ],
  adaptiveDefaultSeconds: 300,
  nominalIntervalSeconds: 300,
  attribution: "test",
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(props: Partial<React.ComponentProps<typeof MeterPopover>> = {}) {
  await act(async () => {
    root.render(<MeterPopover
      usage={{ codex: snapshot(75, WEEK_SECONDS / 2) }}
      registry={registry}
      refreshing={false}
      onRefresh={() => {}}
      onClose={() => {}}
      {...props}
    />);
  });
}

describe("MeterPopover", () => {
  it("renders live windows with pace and resets, and nothing that is not a limit", async () => {
    await mount();
    const text = container.textContent ?? "";
    expect(text).toContain("Codex");
    expect(text).toContain("75% used");
    // 75% spent with half the week gone is ahead of an even pace.
    expect(text).toContain("ahead of pace");
    expect(text).toMatch(/resets in/);
    // No signed pace deltas, no planned-provider matrix, no attribution
    // footer: the card is the limits and only the limits.
    expect(text).not.toMatch(/[+-]\d+%/);
    expect(text).not.toContain("in deficit");
    expect(text).not.toContain("providers planned");
    expect(text).not.toContain("OpenRouter");
    expect(text).not.toContain("CodexBar");
    expect(container.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe("Usage meter");
    // The meter is its own menu-bar window, not a modal over the app: there
    // is nothing behind it to make inert, so claiming modality would mislead
    // a screen reader about what is reachable.
    expect(container.querySelector('[role="dialog"]')?.getAttribute("aria-modal")).toBeNull();
    expect(document.activeElement).toBe(container.querySelector('[role="dialog"]'));
  });

  it("draws a ring gauge for each live provider", async () => {
    await mount({ usage: { codex: snapshot(75, WEEK_SECONDS / 2), claude: snapshot(20, WEEK_SECONDS / 2) } });
    expect(container.querySelector('[aria-label="Codex gauge"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="Claude gauge"]')).not.toBeNull();
    expect(container.querySelector('[aria-label$=" pace"]')).toBeNull();
    // Series colour follows the harness, never its rank.
    expect(container.querySelector(".bg-chart-codex")).not.toBeNull();
    expect(container.querySelector(".bg-chart-claude")).not.toBeNull();
  });

  it("shows every window a provider reports, a reset one as fresh", async () => {
    const codex: UsageSnapshot = {
      windows: [
        { id: "primary", label: "5h", usedPercent: 0, windowMinutes: 300, fresh: true, source: "reported" },
        { id: "secondary", label: "Weekly", usedPercent: 8, windowMinutes: 10_080, resetsInSeconds: WEEK_SECONDS / 2, source: "reported" },
      ],
      planType: "plus",
      source: "reported",
      capturedAt: new Date().toISOString(),
    };
    await mount({ usage: { codex } });
    const text = container.textContent ?? "";
    expect(text).toContain("5h");
    expect(text).toContain("0% used");
    expect(text).toContain("fresh window");
    expect(text).toContain("Weekly");
    expect(text).toContain("8% used");
    expect(text).toContain("plus");
  });

  it("shows registered providers while fresh live usage is loading", async () => {
    await mount({ usage: {} });
    expect(container.textContent).toContain("Codex");
    expect(container.textContent).toContain("Claude");
    expect(container.textContent).toContain("Awaiting live usage");
  });

  it("shows a loading state until the registry arrives", async () => {
    await mount({ usage: {}, registry: null });
    expect(container.textContent).toContain("Loading meter");
  });

  it("expands and collapses a meter bar on click", async () => {
    await mount();
    const bar = container.querySelector<HTMLButtonElement>('[aria-controls="meter-window-weekly"]')!;
    expect(bar.getAttribute("aria-expanded")).toBe("false");
    act(() => { bar.click(); });
    expect(bar.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector("#meter-window-weekly")?.textContent).toContain("75% of this window is used");
    act(() => { bar.click(); });
    expect(bar.getAttribute("aria-expanded")).toBe("false");
  });

  it("refreshes and closes on request", async () => {
    const onRefresh = vi.fn();
    const onClose = vi.fn();
    await mount({ onRefresh, onClose });
    const buttons = [...container.querySelectorAll("button")];
    const refresh = buttons.find(button => button.getAttribute("aria-label") === "Refresh meter")!;
    const close = buttons.find(button => button.getAttribute("aria-label") === "Close meter")!;
    expect(refresh.disabled).toBe(false);
    act(() => { refresh.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    act(() => { close.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(onRefresh).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("disables refresh while a refresh is in flight", async () => {
    await mount({ refreshing: true });
    const refresh = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "Refresh meter")!;
    expect(refresh.disabled).toBe(true);
  });

  it("renders without a registry rather than failing", async () => {
    await mount({ registry: null });
    expect(container.textContent).toContain("Codex");
  });
});
