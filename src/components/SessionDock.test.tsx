// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Code2, FileCode2, TerminalSquare } from "lucide-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_DOCK_WIDTH, defaultDockState, MIN_DOCK_WIDTH, type DockAction, type DockState } from "../dockLayout";
import { DOCK_TAB_LABEL_MIN_WIDTH, SessionDock, type DockPaneDescriptor } from "./SessionDock";

// Contract: testing/feat-dock-shell.md §3.

let container: HTMLDivElement;
let root: Root;

const PANES: DockPaneDescriptor[] = [
  { id: "changes", label: "Changes", icon: FileCode2, available: true, badge: 4 },
  { id: "code", label: "Code", icon: Code2, available: true },
  { id: "terminal", label: "Terminal", icon: TerminalSquare, available: true },
];

// The shipped switcher: seven panes, so the active tab's inline label is the
// last thing that fits beside the Expand and Close buttons. Icons are reused
// from the three above — only the labels and the count matter here.
const ALL_PANES: DockPaneDescriptor[] = [
  { id: "changes", label: "Changes", icon: FileCode2, available: true },
  { id: "code", label: "Code", icon: Code2, available: true },
  { id: "terminal", label: "Terminal", icon: TerminalSquare, available: true },
  { id: "browser", label: "Browser", icon: Code2, available: true },
  { id: "transcript", label: "Transcript", icon: Code2, available: true },
  { id: "tasks", label: "Tasks", icon: Code2, available: true },
  { id: "github", label: "GitHub", icon: Code2, available: true },
];

const open = (overrides: Partial<DockState> = {}): DockState => ({
  ...defaultDockState(),
  open: true,
  pane: "changes",
  visited: ["changes"],
  ...overrides,
});

type MountOptions = {
  state?: DockState;
  panes?: DockPaneDescriptor[];
  sheet?: boolean;
  concealed?: boolean;
  onAction?: (action: DockAction) => void;
  onConnectFolder?: () => void;
};

function mount(options: MountOptions = {}) {
  act(() => {
    root.render(
      <SessionDock
        state={options.state ?? open()}
        panes={options.panes ?? PANES}
        availableWidth={1280}
        sheet={options.sheet ?? false}
        concealed={options.concealed ?? false}
        onAction={options.onAction ?? (() => {})}
        onConnectFolder={options.onConnectFolder}
      >
        {pane => <output data-pane={pane}>{pane} body</output>}
      </SessionDock>,
    );
  });
}

const tabs = () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
const activeTab = () => tabs().find(tab => tab.getAttribute("aria-selected") === "true")!;
const body = (pane: string) => container.querySelector<HTMLElement>(`output[data-pane="${pane}"]`);
const hidden = (element: HTMLElement | null) => !!element?.closest(".hidden");
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
  vi.restoreAllMocks();
});

describe("SessionDock", () => {
  it("renders the switcher as a tablist and the active pane body", () => {
    mount();
    const list = container.querySelector('[role="tablist"][aria-label="Dock panes"]');
    expect(list).not.toBeNull();
    expect(tabs().find(tab => tab.getAttribute("aria-selected") === "true")?.getAttribute("aria-label")).toBe("Changes");
    expect(body("changes")).not.toBeNull();
    expect(hidden(body("changes"))).toBe(false);
  });

  it("hides, never unmounts, a visited pane when switching away", () => {
    mount({ state: open({ pane: "code", visited: ["changes", "code"] }) });
    expect(hidden(body("changes"))).toBe(true);
    expect(body("changes")).not.toBeNull();
    expect(hidden(body("code"))).toBe(false);
  });

  it("does not mount an unvisited pane", () => {
    mount();
    expect(body("code")).toBeNull();
    expect(body("terminal")).toBeNull();
  });

  it("hides the collapsed dock without unmounting visited pane bodies", () => {
    const onAction = vi.fn();
    mount({ state: { ...defaultDockState(), visited: [] }, onAction });
    expect(container.querySelector('[role="tablist"]')).toBeNull();
    expect(container.querySelector("aside")!.classList.contains("hidden")).toBe(true);
    expect(container.querySelectorAll("aside button")).toHaveLength(0);
  });

  it("keeps visited pane bodies mounted while collapsed", () => {
    mount({ state: { ...defaultDockState(), pane: "changes", visited: ["changes"] } });
    expect(body("changes")).not.toBeNull();
    expect(hidden(body("changes"))).toBe(true);
  });

  it("keeps the same body node across expand and restore", () => {
    const state = open();
    mount({ state });
    const before = body("changes");
    mount({ state: { ...state, expanded: true } });
    expect(body("changes")).toBe(before);
    expect(hidden(body("changes"))).toBe(false);
    mount({ state });
    expect(body("changes")).toBe(before);
  });

  it("keeps the same body node across collapse and reopen", () => {
    const state = open();
    mount({ state });
    const before = body("changes");
    mount({ state: { ...state, open: false } });
    expect(body("changes")).toBe(before);
    mount({ state });
    expect(body("changes")).toBe(before);
  });

  it("offers the divider as a real affordance", () => {
    const onAction = vi.fn();
    mount({ onAction });
    const divider = container.querySelector<HTMLElement>('[role="separator"][aria-orientation="vertical"]')!;
    expect(divider.tabIndex).toBe(0);
    expect(divider.getAttribute("aria-valuenow")).toBe(String(DEFAULT_DOCK_WIDTH));
    expect(divider.getAttribute("aria-valuemin")).toBe(String(MIN_DOCK_WIDTH));
    expect(Number(divider.getAttribute("aria-valuemax"))).toBeGreaterThanOrEqual(DEFAULT_DOCK_WIDTH);
    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    });
    expect(onAction).toHaveBeenCalledWith({ type: "set-width", width: DEFAULT_DOCK_WIDTH + 16, available: 1280 });
    act(() => {
      divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(onAction).toHaveBeenCalledWith({ type: "set-width", width: DEFAULT_DOCK_WIDTH, available: 1280 });
  });

  it("hides the divider in sheet mode and offers the scrim instead", () => {
    const onAction = vi.fn();
    mount({ sheet: true, onAction });
    expect(container.querySelector('[role="separator"]')).toBeNull();
    const scrim = container.querySelector<HTMLButtonElement>('button[aria-label="Close dock"].bg-scrim')!;
    click(scrim);
    expect(onAction).toHaveBeenCalledWith({ type: "toggle" });
  });

  it("explains an unavailable pane instead of rendering it", () => {
    const onConnectFolder = vi.fn();
    const panes: DockPaneDescriptor[] = PANES.map(pane => ({
      ...pane,
      available: false,
      unavailableReason: `${pane.label} needs a repository.`,
    }));
    mount({ state: open(), panes, onConnectFolder });
    expect(body("changes")).toBeNull();
    expect(container.textContent).toContain("Changes needs a repository.");
    const connect = [...container.querySelectorAll("button")].find(button => button.textContent === "Connect a folder")!;
    click(connect);
    expect(onConnectFolder).toHaveBeenCalledTimes(1);
  });

  it("rides badges on the switcher whether or not the pane is active", () => {
    mount({ state: open({ pane: "code", visited: ["code"] }) });
    const changesTab = tabs().find(tab => tab.getAttribute("aria-label") === "Changes")!;
    expect(changesTab.textContent).toContain("4");
  });

  it("marks an alerting pane on the open switcher", () => {
    const panes = PANES.map(pane => pane.id === "terminal" ? { ...pane, alert: true } : pane);
    mount({ state: open(), panes });
    expect(container.querySelector('[data-testid="dock-alert-terminal"]')).not.toBeNull();
  });

  it("conceals everything without unmounting when hidden by fullscreen", () => {
    const state = open();
    mount({ state });
    const before = body("changes");
    mount({ state, concealed: true });
    expect(body("changes")).toBe(before);
    expect(hidden(body("changes"))).toBe(true);
    expect(container.querySelector('[role="separator"]')).toBeNull();
  });
  // Seven icon tabs plus an inline label are wider than the room the header
  // leaves beside Expand and Close: at the dock's minimum the label used to
  // spill past the strip's own rounded border and over those buttons.
  it("drops the active tab's inline label at the dock's minimum width", () => {
    mount({ state: open({ pane: "github", visited: ["github"], width: MIN_DOCK_WIDTH }), panes: ALL_PANES });
    expect(activeTab().textContent).toBe("");
    expect(activeTab().getAttribute("aria-label")).toBe("GitHub");
    expect(activeTab().title).toContain("GitHub");
  });

  it("shows the active tab's inline label at the default width", () => {
    mount({ state: open({ pane: "github", visited: ["github"], width: DEFAULT_DOCK_WIDTH }), panes: ALL_PANES });
    expect(activeTab().textContent).toContain("GitHub");
    expect(activeTab().getAttribute("aria-label")).toBe("GitHub");
  });

  it("flips the inline label at the width threshold, not before it", () => {
    const state = open({ pane: "transcript", visited: ["transcript"] });
    mount({ state: { ...state, width: DOCK_TAB_LABEL_MIN_WIDTH - 1 }, panes: ALL_PANES });
    expect(activeTab().textContent).not.toContain("Transcript");
    mount({ state: { ...state, width: DOCK_TAB_LABEL_MIN_WIDTH }, panes: ALL_PANES });
    expect(activeTab().textContent).toContain("Transcript");
  });

  it("keeps the inline label while expanded, where the dock owns the whole split", () => {
    mount({ state: open({ pane: "github", visited: ["github"], width: MIN_DOCK_WIDTH, expanded: true }), panes: ALL_PANES });
    expect(activeTab().textContent).toContain("GitHub");
  });

  it("scrolls the tab strip inside its own border and keeps the header buttons", () => {
    mount({ state: open({ pane: "github", visited: ["github"], width: MIN_DOCK_WIDTH }), panes: ALL_PANES });
    const list = container.querySelector<HTMLElement>('[role="tablist"]')!;
    expect(list.className).toContain("min-w-0");
    expect(list.className).toContain("overflow-x-auto");
    expect(container.querySelector('button[aria-label="Expand dock"]')).not.toBeNull();
    expect(container.querySelector('aside button[aria-label="Close dock"]')).not.toBeNull();
  });
});


describe("dock keyboard navigation", () => {
  it("roves through tabs and connects the active tab to its panel", () => {
    const onAction = vi.fn();
    mount({ onAction });
    expect(tabs().filter(tab => tab.tabIndex === 0)).toHaveLength(1);
    const changes = tabs()[0]; changes.focus();
    act(() => changes.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
    expect(onAction).toHaveBeenLastCalledWith({ type: "open-pane", pane: "code" });
    expect(document.activeElement).toBe(tabs()[1]);
    act(() => tabs()[1].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })));
    expect(onAction).toHaveBeenLastCalledWith({ type: "open-pane", pane: "terminal" });
    expect(document.activeElement).toBe(tabs()[2]);
    act(() => tabs()[2].dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })));
    expect(document.activeElement).toBe(changes);
    expect(document.getElementById(changes.getAttribute("aria-controls")!)?.getAttribute("role")).toBe("tabpanel");
  });
});
