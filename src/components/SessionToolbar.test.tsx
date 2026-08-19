// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Code2, FileCode2, MessageSquareText, TerminalSquare } from "lucide-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SessionToolbar, type SessionToolbarProps } from "./SessionToolbar";

let container: HTMLDivElement;
let root: Root;

const REPO_TABS = [
  { id: "agent", label: "Agent", icon: MessageSquareText },
  { id: "changes", label: "Changes", icon: FileCode2, badge: 2 },
  { id: "code", label: "Code", icon: Code2 },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
];

const noop = () => {};

const props = (overrides: Partial<SessionToolbarProps> = {}): SessionToolbarProps => ({
  title: "Orchestrator",
  tabs: REPO_TABS,
  activeTab: "agent",
  onTabChange: noop,
  model: "Claude Opus",
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

const tabs = () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
const tabByLabel = (label: string) => tabs().find(tab => tab.getAttribute("aria-label") === label)!;
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

  it("labels only the active tab but names every one for assistive tech", () => {
    mount({ activeTab: "code" });
    expect(tabByLabel("Code").textContent).toContain("Code");
    expect(tabByLabel("Terminal").textContent).not.toContain("Terminal");
    expect(tabs().map(tab => tab.getAttribute("aria-label"))).toEqual(["Agent", "Changes", "Code", "Terminal"]);
    expect(tabs().filter(tab => tab.getAttribute("aria-selected") === "true").map(tab => tab.getAttribute("aria-label"))).toEqual(["Code"]);
  });

  it("keeps the changed-file count on the Changes tab while it is inactive", () => {
    mount({ activeTab: "agent" });
    expect(tabByLabel("Changes").textContent).toContain("2");
  });

  it("drops the segmented control for a session with a single panel", () => {
    mount({ tabs: [REPO_TABS[0]] });
    expect(container.querySelector('[role="tablist"]')).toBeNull();
    expect(container.querySelector("h1")!.textContent).toBe("Orchestrator");
  });

  it("reports a tab press", () => {
    const onTabChange = vi.fn();
    mount({ onTabChange });
    click(tabByLabel("Terminal"));
    expect(onTabChange).toHaveBeenCalledWith("terminal");
  });

  it("carries the model and nothing else as quiet context", () => {
    mount();
    expect(container.textContent).toContain("Claude Opus");
    // The branch and its dirty count were the line the user asked to lose; the
    // count already rides on the Changes tab.
    expect(container.textContent).not.toContain("isolated worktree");
    expect(container.textContent).not.toContain("changed");
    expect(container.textContent).not.toMatch(/codex\/|feat\//);
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

  it("leaves the traffic lights their corner in fullscreen", () => {
    mount({ fullscreen: true });
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).toContain("pl-[84px]");
    expect(row.getAttribute("data-tauri-drag-region")).toBe("");
  });
});
