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

  it("names the current view at every window width", () => {
    mount();
    const text = header().textContent ?? "";
    expect(text).not.toContain("bridge");
    expect(text).toContain("Orchestrator");
  });

  it("leaves the traffic lights their corner in a windowed Tauri frame", () => {
    mount();
    expect(header().className).toContain("pl-24");
    expect(header().className).toContain("u-traffic-inset");
  });

  it("can sit flush beside a sidebar without a hairline", () => {
    mount({ flush: true, hideBrand: true });
    expect(header().className).not.toContain("border-b");
    expect(header().className).not.toContain("pl-24");
    expect(header().className).not.toContain("u-traffic-inset");
    expect(header().textContent).not.toContain("bridge");
  });

  it("insets trailing chrome so nested controls can be concentric with the window", () => {
    mount();
    expect(header().className).toContain("pr-[var(--window-control-inset)]");
  });

  it("drops the trailing window-control inset when flush, because the right edge is the rail seam", () => {
    mount({ flush: true });
    expect(header().className).not.toContain("pr-[var(--window-control-inset)]");
    expect(header().className).toContain("pr-3");
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

  it("carries the sidebar's own controls, and its traffic-light corner, while the rail is hidden", () => {
    mount({
      flush: true,
      hideBrand: true,
      sidebarHidden: true,
      leading: <button type="button">Show sidebar</button>,
    });
    expect(header().className).toContain("u-traffic-inset");
    expect(header().className).toContain("pl-24");
    // First in the row, and desktop-only — the drawer keeps its own toggle below sm.
    const cluster = header().firstElementChild as HTMLElement;
    expect(cluster.textContent).toBe("Show sidebar");
    expect(cluster.className).toContain("hidden");
    expect(cluster.className).toContain("sm:flex");
    // Still the window's grab handle with a control sitting on it.
    expect(header().getAttribute("data-tauri-drag-region")).toBe("deep");
  });

  it("renders no leading cluster while the rail owns the leading edge", () => {
    mount({ flush: true, hideBrand: true });
    expect(header().querySelector("button[type='button']:not([aria-label])")).toBeNull();
    expect(header().className).not.toContain("pl-24");
  });

  it("docks the actions cluster at the right edge", () => {
    mount({ actions: <button type="button">Agent Fleet</button> });
    expect(header().textContent).toContain("Agent Fleet");
  });
});
