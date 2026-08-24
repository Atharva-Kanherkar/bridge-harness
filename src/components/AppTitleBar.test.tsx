// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppTitleBar, type AppTitleBarProps } from "./AppTitleBar";

let container: HTMLDivElement;
let root: Root;

const props = (overrides: Partial<AppTitleBarProps> = {}): AppTitleBarProps => ({
  title: "Orchestrator",
  navOpen: false,
  onOpenNav: () => {},
  ...overrides,
});

function mount(overrides: Partial<AppTitleBarProps> = {}) {
  act(() => {
    root.render(<AppTitleBar {...props(overrides)} />);
  });
}

const header = () => container.querySelector("header")!;

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

describe("AppTitleBar", () => {
  it("drags the window from anywhere on the strip", () => {
    mount();
    expect(header().getAttribute("data-tauri-drag-region")).toBe("deep");
  });

  it("carries the brand on desktop and the view title where the sidebar is hidden", () => {
    mount();
    const text = header().textContent ?? "";
    expect(text).toContain("bridge");
    expect(text).toContain("Orchestrator");
  });

  it("leaves the traffic lights their corner", () => {
    mount();
    expect(header().className).toContain("pl-24");
  });

  it("can sit flush beside a sidebar without a hairline", () => {
    mount({ flush: true, hideBrand: true });
    expect(header().className).not.toContain("border-b");
    expect(header().className).not.toContain("pl-24");
    expect(header().textContent).not.toContain("bridge");
  });

  it("insets trailing chrome so nested controls can be concentric with the window", () => {
    mount();
    expect(header().className).toContain("pr-[var(--window-control-inset)]");
  });

  it("opens navigation from the mobile toggle", () => {
    const onOpenNav = vi.fn();
    mount({ onOpenNav });
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="Open navigation"]')!;
    act(() => {
      toggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onOpenNav).toHaveBeenCalledTimes(1);
  });

  it("docks the actions cluster at the right edge", () => {
    mount({ actions: <button type="button">Mission Control</button> });
    expect(header().textContent).toContain("Mission Control");
  });
});
