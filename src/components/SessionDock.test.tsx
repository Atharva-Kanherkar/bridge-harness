// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Code2, FileCode2, TerminalSquare } from "lucide-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_DOCK_WIDTH, defaultDockState, type DockAction, type DockState } from "../dockLayout";
import { SessionDock, type DockPaneDescriptor } from "./SessionDock";

// Contract: testing/feat-dock-shell.md §3.

let container: HTMLDivElement;
let root: Root;

const PANES: DockPaneDescriptor[] = [
  { id: "changes", label: "Changes", icon: FileCode2, available: true, badge: 4 },
  { id: "code", label: "Code", icon: Code2, available: true },
  { id: "terminal", label: "Terminal", icon: TerminalSquare, available: true },
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

  it("renders the collapsed rail with every pane one click away", () => {
    const onAction = vi.fn();
    mount({ state: { ...defaultDockState(), visited: [] }, onAction });
    expect(container.querySelector('[role="tablist"]')).toBeNull();
    const rail = [...container.querySelectorAll<HTMLButtonElement>("aside button")];
    expect(rail.map(button => button.getAttribute("aria-label"))).toEqual(["Changes", "Code", "Terminal"]);
    click(rail[2]);
    expect(onAction).toHaveBeenCalledWith({ type: "open-pane", pane: "terminal" });
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

  it("rides badges on the switcher and the rail", () => {
    mount({ state: open({ pane: "code", visited: ["code"] }) });
    const changesTab = tabs().find(tab => tab.getAttribute("aria-label") === "Changes")!;
    expect(changesTab.textContent).toContain("4");
    mount({ state: { ...defaultDockState(), visited: [] } });
    const railChanges = container.querySelector<HTMLButtonElement>('aside button[aria-label="Changes"]')!;
    expect(railChanges.querySelector("span.rounded-full")).not.toBeNull();
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
});
