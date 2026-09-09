// @vitest-environment jsdom
// The meter popover contract: live provider windows render with usage bars,
// reset countdowns, and CodexBar pace lines; planned providers are listed, not
// hidden; refresh and close are wired.
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
  it("renders live windows with pace, resets, and the planned matrix", async () => {
    await mount();
    const text = container.textContent ?? "";
    expect(text).toContain("Codex");
    expect(text).toContain("75% used");
    expect(text).toContain("in deficit");
    expect(text).toMatch(/resets in/);
    expect(text).toContain("1 more providers planned");
    expect(text).toContain("OpenRouter");
    expect(container.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe("Usage meter");
    expect(container.querySelector('[role="dialog"]')?.getAttribute("aria-modal")).toBe("true");
    expect(document.activeElement).toBe(container.querySelector('[role="dialog"]'));
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
