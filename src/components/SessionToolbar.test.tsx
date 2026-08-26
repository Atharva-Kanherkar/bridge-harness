// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SessionToolbar, type SessionToolbarProps } from "./SessionToolbar";

let container: HTMLDivElement;
let root: Root;

const noop = () => {};

const props = (overrides: Partial<SessionToolbarProps> = {}): SessionToolbarProps => ({
  title: "Orchestrator",
  modelControl: <span>Claude Opus</span>,
  dockOpen: false,
  onToggleDock: noop,
  browserOpen: false,
  onToggleBrowser: noop,
  fullscreen: false,
  onToggleFullscreen: noop,
  ...overrides,
});

function mount(overrides: Partial<SessionToolbarProps> = {}) {
  act(() => {
    root.render(<SessionToolbar {...props(overrides)} />);
  });
}

const overflow = () => container.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]')!;
const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
const click = (element: Element) => {
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
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
  document.querySelectorAll('[role="menu"]').forEach(node => node.remove());
  vi.restoreAllMocks();
});

describe("SessionToolbar", () => {
  it("states the title once, with no second meta line under it", () => {
    mount();
    expect(container.querySelectorAll("h1")).toHaveLength(1);
    expect(container.querySelector("h1")!.textContent).toBe("Orchestrator");
    // The old header printed the harness twice: "Orchestrator · Claude · Claude Opus".
    expect(container.textContent).not.toContain("Orchestrator · Claude");
  });

  it("carries no tablist — the panel switcher lives in the dock now", () => {
    mount();
    expect(container.querySelector('[role="tablist"]')).toBeNull();
  });

  it("offers the dock toggle and reflects its state", () => {
    const onToggleDock = vi.fn();
    mount({ onToggleDock });
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="Toggle dock"]')!;
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    click(toggle);
    expect(onToggleDock).toHaveBeenCalledTimes(1);
    mount({ dockOpen: true });
    expect(container.querySelector('button[aria-label="Toggle dock"]')!.getAttribute("aria-pressed")).toBe("true");
  });

  it("carries the model and nothing else as quiet context", () => {
    mount();
    expect(container.textContent).toContain("Claude Opus");
    // The branch and its dirty count were the line the user asked to lose; the
    // count rides on the dock's Changes tab.
    expect(container.textContent).not.toContain("isolated worktree");
    expect(container.textContent).not.toContain("changed");
    expect(container.textContent).not.toMatch(/codex\/|feat\//);
  });

  it("offers search for this chat when the callback exists", () => {
    mount();
    expect(container.querySelector('button[aria-label="Search this chat"]')).toBeNull();
    mount({ onToggleRecall: noop, recallOpen: true });
    const search = container.querySelector<HTMLButtonElement>('button[aria-label="Search this chat"]')!;
    expect(search.getAttribute("aria-pressed")).toBe("true");
  });

  it("collects the window actions behind one overflow control", () => {
    mount();
    expect(menu()).toBeNull();
    click(overflow());
    const text = menu()!.textContent ?? "";
    expect(text).toContain("Browser");
    expect(text).toContain("Fullscreen");
    // No End for a session that is not live, and no router entry without a repo.
    expect(text).not.toContain("End session");
    expect(text).not.toContain("Learning router");
  });

  it("offers End and the router only when those callbacks exist", () => {
    mount({ onEnd: noop, onOpenRouterSettings: noop });
    click(overflow());
    expect(menu()!.textContent).toContain("End session");
    expect(menu()!.textContent).toContain("Learning router");
  });

  it("disables End while a turn is in flight", () => {
    mount({ onEnd: noop, busy: true });
    click(overflow());
    const end = [...menu()!.querySelectorAll("button")].find(button => button.textContent?.includes("End session"))!;
    expect(end.disabled).toBe(true);
  });

  it("checks Browser in the menu while the browser is open", () => {
    mount({ browserOpen: true });
    click(overflow());
    const browser = [...menu()!.querySelectorAll('[role="menuitemcheckbox"]')].find(item => item.textContent?.includes("Browser"))!;
    expect(browser.getAttribute("aria-checked")).toBe("true");
  });

  it("does not reserve the traffic-light corner; the title bar above owns it", () => {
    mount({ fullscreen: true });
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).not.toContain("pl-24");
    expect(row.getAttribute("data-tauri-drag-region")).toBe("deep");
  });

  it("stays a whole-row window drag handle when windowed", () => {
    mount();
    const row = container.firstElementChild as HTMLElement;
    expect(row.getAttribute("data-tauri-drag-region")).toBe("deep");
  });

  it("carries select-none so a mis-started drag never selects the title", () => {
    mount();
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).toContain("select-none");
  });

  it("renders actions children in the right cluster", () => {
    mount({ actions: <button type="button">Usage</button> });
    expect(container.textContent).toContain("Usage");
  });

  it("renders the bypass badge through props with its full wording intact", () => {
    const onOpenSettings = vi.fn();
    mount({
      bypassBadge: (
        <button type="button" onClick={onOpenSettings}>Approvals bypassed</button>
      ),
    });
    const badge = [...container.querySelectorAll("button")].find(button => button.textContent === "Approvals bypassed")!;
    expect(badge).toBeTruthy();
    click(badge);
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it("hides the mobile nav button when no handler is given", () => {
    mount();
    expect(container.querySelector('button[aria-label="Open navigation"]')).toBeNull();
  });

  it("shows the mobile-only nav button when onOpenNav is provided", () => {
    const onOpenNav = vi.fn();
    mount({ onOpenNav, navOpen: true });
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="Open navigation"]')!;
    expect(toggle.className).toContain("sm:hidden");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    click(toggle);
    expect(onOpenNav).toHaveBeenCalledTimes(1);
  });
});
