// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
// xterm paints to canvas, which jsdom lacks; the pane's own suite covers the
// terminal wiring, and here it only needs to mount.
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    open() {}
    loadAddon() {}
    dispose() {}
    onData() { return { dispose() {} }; }
    write() {}
    writeln() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

// Spy on the greeting picker without changing its behaviour, so a test can
// assert what the welcome hero feeds it (the owning project name, not the
// workspace title) regardless of which line the picker lands on.
vi.mock("./greetings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./greetings")>();
  return { ...actual, pickGreeting: vi.fn(actual.pickGreeting) };
});

import { App, ChatModelControl } from "./App";
import { pickGreeting } from "./greetings";
// Pre-resolve the lazy pane chunks so Suspense settles inside act().
import "./components/TerminalPane";
import "./components/CodePanel";
import type { AdapterDescriptor } from "./types";
import { bridgeApi } from "./api";
import { SHORTCUTS } from "./keymap";

const adapters: AdapterDescriptor[] = [
  {
    id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "gpt-balanced", label: "GPT Balanced", tier: "standard", defaultForTier: true }], defaultModel: "gpt-balanced",
  },
  {
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: true }], defaultModel: "opus",
  },
];

describe("ChatModelControl", () => {
  it("shows the exact orchestrator runtime and explains a model switch", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const onChange = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" roleLabel="Orchestrator" compact onChange={onChange} />));

    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Orchestrator model: Codex GPT Balanced"]')!;
    expect(trigger.textContent).toContain("Codex · GPT Balanced");
    await act(async () => trigger.click());
    expect(container.textContent).toContain("Switching restarts the provider session. History stays.");

    const opus = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Claude Opus"))!;
    await act(async () => opus.click());
    expect(onChange).toHaveBeenCalledWith("claude", "opus");
    await act(async () => root.unmount());
  });
});

// The Cursor sidebar mock rendered invented repos and no session list. The
// real rail is the only rail; this guard keeps the mock from coming back.
describe("shell flags", () => {
  it("ships the real rail, not the Cursor sidebar mock", () => {
    const source = readFileSync(join(__dirname, "App.tsx"), "utf8");
    expect(source).not.toContain("SHOW_CURSOR_SIDEBAR_MOCK");
    expect(source).not.toContain("CursorSidebarMock");
    expect(source).not.toContain("RightRailPreview");
    expect(source).not.toContain("NewChatDialog");
    expect(source).toContain("BridgeSidebar");
    expect(source).toContain('onOpenMarketplace={() => setView("marketplace")}');
    expect(source).toContain('onOpenMemory={() => setView("memory")}');
    expect(source).toContain('if (view !== "memory") setMemoryDraft(null)');
    expect(source).not.toContain('setView("automations")');
    expect(source).not.toContain('view === "automations"');
    expect(source).toContain("chromeFullscreen");
    expect(source).toContain("data-flush-window");
    expect(source).toContain("setLayoutFullscreenDocument");
    expect(source).toContain("notifyLayoutFullscreen");
    expect(source).not.toContain("chromeFullscreen && <AppTitleBar");
    // A hidden rail has no header to hold them, so the chrome row does — and
    // the panel button is then the only pointer route back to the sidebar.
    expect(source).toContain("WindowHistoryChevrons");
    expect(source).toContain("WindowPanelButton");
    expect(source).toContain("sidebarHidden={sidebarCollapsed}");
    expect(source).not.toContain('paradigm === "grid" ? "Focus" : "Agent Fleet"');
    expect(source).toContain("showWindowNav");
    expect(source).toContain("flex h-[100dvh] flex-row");
  });

  it("does not statically import the look-at preview in the production entry", () => {
    const source = readFileSync(join(__dirname, "main.tsx"), "utf8");
    expect(source).not.toMatch(/^import \{ RightRailPreview \}/m);
    expect(source).toContain('import("./previews/RightRailPreview")');
  });
});

// ── The dock in the real App ─────────────────────────────────────────────────
// Contract: testing/feat-dock-shell.md §4 and §5. These mount the whole App on
// the mock api. jsdom has no ResizeObserver, so a controllable stand-in drives
// the section width the sheet threshold reads.

let observedWidth = 1280;
const resizeObservers = new Set<MockResizeObserver>();
class MockResizeObserver {
  callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    resizeObservers.add(this);
  }
  observe() {
    this.callback([{ contentRect: { width: observedWidth } } as ResizeObserverEntry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {
    resizeObservers.delete(this);
  }
}
function fireSectionWidth(width: number) {
  observedWidth = width;
  act(() => {
    resizeObservers.forEach(observer =>
      observer.callback([{ contentRect: { width } } as ResizeObserverEntry], observer as unknown as ResizeObserver));
  });
}

let container: HTMLDivElement;
let root: Root;

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) {
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 0));
    });
  }
}

async function mountApp() {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
}

const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')];
const dockToggle = () => container.querySelector<HTMLButtonElement>('button[aria-label="Toggle dock"]');
const dockAside = () => container.querySelector<HTMLElement>('aside[aria-label="Dock"]');
const composer = () => container.querySelector<HTMLTextAreaElement>("textarea");
const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await settle(2);
};
const key = async (init: KeyboardEventInit) => {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init }));
  });
  await settle(1);
};
const chord = { altKey: true, metaKey: true };

/** Open a workspace session identified by its dirty-file pill ("4 files" is
 * demo-1, "7 files" is demo-2 in the mock state). */
async function openWorkspaceSession(pill: string) {
  for (const row of chatRows()) {
    await click(row);
    if (container.textContent?.includes(pill)) return;
  }
  throw new Error(`no session showed "${pill}"`);
}

describe("the dock in the session view", () => {
  beforeAll(async () => {
    vi.stubGlobal("ResizeObserver", MockResizeObserver);
    await bridgeApi.resetModelProfiles();
  });

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    // The environment's storage shim is read-only-ish (no clear/removeItem), so
    // each test gets a fresh full stub, the same way the sidebar tests do.
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, value); },
        removeItem: (key: string) => { store.delete(key); },
        clear: () => store.clear(),
      },
    });
    observedWidth = 1280;
  });

  afterEach(async () => {
    if (root) await act(async () => root.unmount());
    container?.remove();
    resizeObservers.clear();
  });
  // The mock state's workspace ("Build session supervisor") sits under a project
  // named "Bridge"; the hero must name the project, not the workspace's own title.
  it("feeds the welcome hero its owning project name, not the workspace title", async () => {
    vi.mocked(pickGreeting).mockClear();
    // Select the mock workspace ("Build session supervisor", project "Bridge")
    // as the welcome target, the way returning to a repo would.
    localStorage.setItem("bridge.chat.lastWorkspaceId", "demo-1");
    await mountApp();
    const welcomeCalls = vi.mocked(pickGreeting).mock.calls.filter(args => args[0] === "welcome");
    expect(welcomeCalls.length).toBeGreaterThan(0);
    // Once the workspace resolves, the hero is fed the owning project ("Bridge").
    expect(welcomeCalls.at(-1)?.[1]).toBe("Bridge");
    // And it is never fed the workspace's own title, on any render.
    expect(welcomeCalls.some(args => args[1] === "Build session supervisor")).toBe(false);
  });

  it("services a tray refresh without raising the app or opening an in-app meter", async () => {
    let trayAction: ((action: "refresh") => void) | undefined;
    const onTray = vi.spyOn(bridgeApi, "onMeterTray").mockImplementation(async handler => {
      trayAction = handler;
      return () => undefined;
    });
    const reveal = vi.spyOn(bridgeApi, "revealMainWindow").mockResolvedValue();
    const refresh = vi.spyOn(bridgeApi, "refreshMeter").mockResolvedValue();
    await mountApp();

    await act(async () => { trayAction?.("refresh"); });
    await settle(2);

    expect(refresh).toHaveBeenCalledTimes(1);
    // The tray must not pull the main window forward — that is what made a
    // menu-bar click feel like it opened "in the app", and the reveal it
    // attempted was the ungranted `window.show` IPC behind the error banner.
    expect(reveal).not.toHaveBeenCalled();
    // The meter is a separate menu-bar window now. Nothing renders it over
    // the app, so no dialog may appear here at all.
    expect(container.querySelector('[role="dialog"][aria-label="Usage meter"]')).toBeNull();
    onTray.mockRestore();
    reveal.mockRestore();
    refresh.mockRestore();
  });


  it("cancels new orchestrator setup without creating a session", async () => {
    await mountApp();
    await click(container.querySelector<HTMLButtonElement>('button[aria-label="Projects"]')!);
    const create = vi.spyOn(bridgeApi, "createWorkspaceSession");
    const open = () => click([...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === "New agent")!);
    await open();
    expect(container.querySelector('[aria-labelledby="orchestrator-create-title"]')).not.toBeNull();
    await click(container.querySelector<HTMLButtonElement>('button[aria-label="Cancel new orchestrator"]')!);
    expect(create).not.toHaveBeenCalled();
    await open();
    await key({ key: "Escape" });
    expect(create).not.toHaveBeenCalled();
    expect(container.querySelector('[aria-labelledby="orchestrator-create-title"]:not([aria-hidden="true"])')).toBeNull();
    create.mockRestore();
  });

  it("keeps the conversation and an open pane on screen together, with the composer usable", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");

    const toolbar = container.querySelector("h1")!.parentElement!;
    expect(toolbar.querySelector('[role="tablist"]')).toBeNull();
    const toggle = dockToggle()!;
    expect(toggle.getAttribute("aria-pressed")).toBe("false");

    await click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector('[role="tablist"][aria-label="Dock panes"]')).not.toBeNull();
    expect(container.textContent).toContain("CHANGES");
    expect(composer()).not.toBeNull();
    expect(composer()!.disabled).toBe(false);
  });

  it("keys dock state to the workspace", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("true");

    await openWorkspaceSession("7 files");
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("false");

    await openWorkspaceSession("4 files");
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("true");
  });

  it("dims repo panes in a direct chat and explains why", async () => {
    await mountApp();
    await act(async () => {
      await bridgeApi.createChat("codex", null, "Scratch questions");
    });
    await settle();
    const scratch = chatRows().find(row => row.title.includes("Scratch questions"))!;
    await click(scratch);

    await key({ ...chord, code: "Digit1", key: "1" });
    expect(container.textContent).toContain("Changes needs a repository.");
    expect(container.textContent).not.toContain("CHANGES");
  });

  it("conceals the dock in fullscreen without destroying it", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    const bodyBefore = container.querySelector('aside[aria-label="Dock"] .h-full > *');
    expect(bodyBefore).not.toBeNull();

    await key({ ...chord, key: "f" });
    expect(dockAside()!.classList.contains("hidden")).toBe(true);
    expect(container.querySelector('aside[aria-label="Dock"] .h-full > *')).toBe(bodyBefore);

    await key({ ...chord, key: "f" });
    expect(dockAside()!.classList.contains("hidden")).toBe(false);
  });

  it("renders the open dock as a sheet below the split threshold", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    expect(container.querySelector('[aria-label="Resize dock"]')).not.toBeNull();

    fireSectionWidth(600);
    expect(container.querySelector('[aria-label="Resize dock"]')).toBeNull();
    const scrim = container.querySelector<HTMLButtonElement>("button.bg-scrim")!;
    expect(scrim.getAttribute("aria-label")).toBe("Close dock");
    await click(scrim);
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("false");
  });

  it("answers the dock chords", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");

    await key({ ...chord, code: "Enter", key: "Enter" });
    expect(container.querySelector('button[aria-label="Restore dock"]')).toBeNull();

    await key({ ...chord, code: "Digit0", key: "0" });
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("true");

    await key({ ...chord, code: "Digit2", key: "2" });
    const codeTab = [...container.querySelectorAll('[role="tab"]')].find(tab => tab.getAttribute("aria-label") === "Code")!;
    expect(codeTab.getAttribute("aria-selected")).toBe("true");

    await key({ ...chord, code: "Enter", key: "Enter" });
    expect(container.querySelector('button[aria-label="Restore dock"]')).not.toBeNull();
    await key({ ...chord, code: "Enter", key: "Enter" });
    expect(container.querySelector('button[aria-label="Expand dock"]')).not.toBeNull();

    await key({ ...chord, code: "Digit0", key: "0" });
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("false");
  });

  // Contract: testing/feat-dock-changes.md §5.
  it("quotes a file and a hunk from the diff into the composer", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit1", key: "1" });
    await settle(3);

    const dock = dockAside()!;
    await click(dock.querySelector('button[aria-label="Reference src-tauri/bridge-core/src/policy.rs in the composer"]')!);
    const textarea = composer()!;
    expect(textarea.value).toContain("@src-tauri/bridge-core/src/policy.rs");
    expect(document.activeElement).toBe(textarea);

    await click(dock.querySelector('button[aria-expanded="false"]')!);
    await click(dock.querySelector('button[aria-label="Reference lines 10-30 in the composer"]')!);
    expect(textarea.value).toMatch(/lines 10-30 $/);
  });

  it("opens a file from the diff in the Code pane", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit1", key: "1" });
    await settle(3);

    await click(dockAside()!.querySelector('button[aria-label="Open src-tauri/bridge-core/src/policy.rs in the Code pane"]')!);
    await settle(4);
    const codeTab = [...container.querySelectorAll('[role="tab"]')].find(tab => tab.getAttribute("aria-label") === "Code")!;
    expect(codeTab.getAttribute("aria-selected")).toBe("true");
    expect([...container.querySelectorAll("button[title]")].some(node => node.getAttribute("title") === "src-tauri/bridge-core/src/policy.rs")).toBe(true);
  });

  // Contract: testing/feat-dock-code.md §4.
  it("turns a sent mention into a live link back into the editor", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");

    const textarea = composer()!;
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(textarea, "please recheck @src/App.tsx before merging");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    await settle(6);

    const mention = container.querySelector<HTMLButtonElement>('button[aria-label="Open src/App.tsx in the Code pane"]');
    expect(mention).not.toBeNull();
    await click(mention!);
    await settle(4);
    const codeTab = [...container.querySelectorAll('[role="tab"]')].find(tab => tab.getAttribute("aria-label") === "Code")!;
    expect(codeTab.getAttribute("aria-selected")).toBe("true");
    expect([...container.querySelectorAll("button[title]")].some(node => node.getAttribute("title") === "src/App.tsx")).toBe(true);
  });

  // Contract: testing/feat-dock-transcript.md §4.
  it("opens the transcript with the session's replayed events, in repo sessions and direct chats", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit5", key: "5" });
    await settle(3);
    expect(container.textContent).toContain("tool.started");

    await act(async () => {
      await bridgeApi.createChat("codex", null, "Transcript scratch");
    });
    await settle();
    const scratch = chatRows().find(row => row.title.includes("Transcript scratch"))!;
    await click(scratch);
    await key({ ...chord, code: "Digit5", key: "5" });
    await settle(2);
    expect(dockAside()!.textContent).not.toContain("needs a repository");
    expect(dockAside()!.textContent).toContain("events");
  });

  it("reveals a forest entry into the conversation", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function () {
      scrolled.push((this as HTMLElement).id);
    };
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit5", key: "5" });
    await settle(3);
    const entriesTab = [...dockAside()!.querySelectorAll("button")].find(button => button.textContent === "entries")!;
    await click(entriesTab);
    const reveal = dockAside()!.querySelector<HTMLButtonElement>('button[aria-label^="Reveal entry"]')!;
    const entryId = reveal.getAttribute("aria-label")!.match(/^Reveal entry (\S+) /)![1];
    await click(reveal);
    await act(async () => {
      await new Promise(resolve => requestAnimationFrame(() => resolve(undefined)));
    });
    expect(scrolled).toContain(`forest-entry-${entryId}`);
  });

  it("serves per-session slices from the mock replay", async () => {
    const all = await bridgeApi.replaySessionEvents("session-1", 0, undefined, false);
    expect(all.length).toBeGreaterThan(0);
    expect(all.every(event => event.sessionId === "session-1")).toBe(true);
    expect([...all].sort((a, b) => a.sequence - b.sequence)).toEqual(all);

    const tail = await bridgeApi.replaySessionEvents("session-1", 0, 1, true);
    expect(tail).toHaveLength(1);
    expect(tail[0].sequence).toBe(all[all.length - 1].sequence);

    const after = await bridgeApi.replaySessionEvents("session-1", all[0].sequence, undefined, false);
    expect(after.every(event => event.sequence > all[0].sequence)).toBe(true);
  });

  // Contract: testing/feat-dock-browser.md §3.
  it("opens the browser pane from the toolbar menu and keeps it across pane switches", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(container.querySelector('button[aria-label="Session actions"]')!);
    const item = [...document.querySelectorAll('[role="menu"] [role="menuitemcheckbox"]')].find(node => node.textContent?.includes("Browser"))!;
    await click(item);
    await settle(2);

    const browserTab = [...container.querySelectorAll('[role="tab"]')].find(tab => tab.getAttribute("aria-label") === "Browser")!;
    expect(browserTab.getAttribute("aria-selected")).toBe("true");

    // The checkbox closes what it opened: a second activation collapses the dock.
    await click(container.querySelector('button[aria-label="Session actions"]')!);
    const again = [...document.querySelectorAll('[role="menu"] [role="menuitemcheckbox"]')].find(node => node.textContent?.includes("Browser"))!;
    await click(again);
    await settle(2);
    expect(dockToggle()!.getAttribute("aria-pressed")).toBe("false");
    await click(dockToggle()!);
    // The extension-based BrowserSurface is paused; the dock's "browser" pane
    // now renders the plain iframe-based SimpleBrowser.
    const surface = () => [...dockAside()!.querySelectorAll("*")].find(node => node.textContent === "No page open");
    const before = surface();
    expect(before).toBeTruthy();

    await key({ ...chord, code: "Digit1", key: "1" });
    expect(surface()).toBe(before);
  });

  it("gives direct chats a browser pane, not an excuse", async () => {
    await mountApp();
    await act(async () => {
      await bridgeApi.createChat("codex", null, "Browser scratch");
    });
    await settle();
    await click(chatRows().find(row => row.title.includes("Browser scratch"))!);
    await key({ ...chord, code: "Digit4", key: "4" });
    await settle(2);
    expect(dockAside()!.textContent).not.toContain("needs a repository");
    expect(dockAside()!.textContent).toContain("No page open");
  });

  // Contract: testing/feat-dock-terminal.md §4.
  it("opens the multi-shell terminal pane on the third chord", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit3", key: "3" });
    await settle(4);
    if (!dockAside()!.querySelector('button[aria-label="New shell"]')) {
      const html = dockAside()!.innerHTML;
      const at = html.indexOf("min-h-0 flex-1");
      console.log("PANE HTML:", html.slice(at, at + 700));
    }
    expect(dockAside()!.querySelector('button[aria-label="New shell"]')).not.toBeNull();
    expect(dockAside()!.textContent).toContain("MB scrollback");
  });

  // Contract: testing/feat-dock-tasks.md §4.
  it("opens the tasks pane on the sixth chord with the live roster", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit6", key: "6" });
    await settle(3);
    const dock = dockAside()!;
    expect(dock.textContent).toContain("WORKING");
    expect(dock.textContent).toContain("implementation");
    expect(dock.textContent).toContain("DONE");
    expect(dock.textContent).toContain("Update the auth serializer");
    expect(dock.textContent).toContain("owned_path_conflict");
  });

  it("carries the running count on the tasks descriptor before the pane ever mounts", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    const tasksTab = [...container.querySelectorAll('[role="tab"]')].find(tab => tab.getAttribute("aria-label") === "Tasks")!;
    expect(tasksTab.textContent).toContain("1");
  });

  it("mounts the usage dot beside a worker's steer composer", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    await key({ ...chord, code: "Digit6", key: "6" });
    await settle(3);
    const openWorker = dockAside()!.querySelector<HTMLButtonElement>('button[aria-label="Open worker Implementation · strong"]')!;
    await click(openWorker);

    expect(container.querySelector("h1")!.textContent).toContain("Implementation");
    expect(container.textContent).toContain("This is a background worker");
    expect(container.querySelector('[aria-label^="Open usage"]')).not.toBeNull();
  });

  it("keeps the usage dot out of the title bar", async () => {
    await mountApp();
    expect(container.querySelector("header")).not.toBeNull();
    expect(container.querySelector('header [aria-label^="Open usage"]')).toBeNull();
  });

  it("mounts the usage dot at the chat composer's leading edge", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const dot = container.querySelector<HTMLButtonElement>('[data-composer-frame] [aria-label^="Open usage"]');
    expect(dot).not.toBeNull();
    expect(dot!.getAttribute("aria-controls")).toBe("usage-dot-panel");
  });

  it("lets Escape restore an expanded pane before it leaves fullscreen", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit0", key: "0" });
    await key({ ...chord, code: "Enter", key: "Enter" });
    expect(container.querySelector('button[aria-label="Restore dock"]')).not.toBeNull();

    await key({ ...chord, key: "f" });
    // Fullscreen chrome is signalled on the shell root now, not toolbar padding.
    const shell = () => container.querySelector("[data-fullscreen]");
    expect(shell()).not.toBeNull();

    // The dock is concealed in fullscreen, so the restore is observed through
    // the persisted layout: the first Escape lands on the expand, not fullscreen.
    await key({ key: "Escape" });
    expect(JSON.parse(localStorage.getItem("bridge.dock.v1.demo-1")!).expanded).toBe(false);
    expect(shell()).not.toBeNull();

    await key({ key: "Escape" });
    expect(shell()).toBeNull();
  });

  // Contract: testing/feat-360-session-shell-cold-start.md §Part 1.
  // AppTitleBar is the only <header> in the flush/no-brand form the shell
  // mounts it in; other components (worker cards, plan cards, …) also render
  // a bare <header>, so this pins on the flush-specific class instead.
  const appTitleBar = () => container.querySelector('header[data-tauri-drag-region="deep"]');

  it("collapses to one chrome row on the session view — no second AppTitleBar strip", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    expect(appTitleBar()).toBeNull();
    expect(container.querySelector('button[aria-label="Toggle dock"]')).not.toBeNull();
  });

  it("carries the model picker only in the composer, not the toolbar", async () => {
    await mountApp();
    await openWorkspaceSession("7 files");
    // The toolbar no longer duplicates the composer's model control; the sole
    // picker lives below and opens upward, into the view.
    const pickers = [...container.querySelectorAll<HTMLButtonElement>('button[aria-label*="model:"]')];
    expect(pickers).toHaveLength(1);
    await click(pickers[0]);
    const panel = container.querySelector<HTMLElement>(".u-glass-popover")!;
    expect(panel).not.toBeNull();
    expect(panel.className).toContain("bottom-full");
  });

  // Contract: testing/feat-aside-chat.md. A `$harness` shortcut typed inside
  // an open chat is a delegation, not a navigation: the aside floats over the
  // conversation, which stays selected and untouched underneath.
  it("opens a $harness shortcut as an aside over the current chat", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const titleBefore = container.querySelector("h1")!.textContent;
    const box = composer()!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "$claude is the plan sound?");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    await settle(4);
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')!;
    expect(aside).not.toBeNull();
    expect(aside.textContent).toContain("is the plan sound?");
    // The chat underneath never moved.
    expect(container.querySelector("h1")!.textContent).toBe(titleBefore);

    // Promote: the aside becomes the active chat and the panel goes away.
    const promote = [...aside.querySelectorAll("button")].find(button => button.textContent?.includes("Open as chat"))!;
    await click(promote);
    expect(document.body.querySelector('div[role="dialog"][aria-label^="Aside"]')).toBeNull();
    // The browser mock has no native title resolver. Keep its placeholder rather
    // than treating the full first message as an explicitly chosen chat name.
    expect(container.querySelector("h1")!.textContent).toBe("New aside");
  });

  // Contract: testing/fix-side-chat-model.md. A side chat begins on a resolved
  // model, not the bare adapter default. Opened from a Claude chat, a $codex
  // aside must start on Codex's Standard model (GPT Terra), where the old code
  // used Codex's Fast defaultModel (GPT Luna).
  it("begins a $codex side chat on the Standard model, not the Fast default", async () => {
    await mountApp();
    // Re-query the composer each time: the welcome textarea unmounts once the
    // first shortcut opens a session and a fresh session composer takes over.
    const type = async (text: string) => {
      const box = composer()!;
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
        setter.call(box, text);
        box.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
      await settle(4);
    };
    // A Claude chat to consult from, then a Codex side chat over it.
    await type("$claude review the plan");
    const createSpy = vi.spyOn(bridgeApi, "createAsideChat");
    await type("$codex sanity check");
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Codex"]');
    expect(aside).not.toBeNull();
    // The prompt is conversation content, not a permanent user-chosen title.
    expect(createSpy).toHaveBeenCalledWith(expect.any(String), "codex", "gpt-5.6-terra", null);
  });

  // Contract: testing/fix-side-chat-model.md. The header picker switches the
  // side chat's own model through update_chat_model, never the chat underneath.
  it("switches the side chat's model through update_chat_model on the aside session", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const box = composer()!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "$claude is the plan sound?");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    await settle(8);
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')!;
    const pill = aside.querySelector<HTMLButtonElement>('[aria-label^="Aside model:"]')!;
    expect(pill).not.toBeNull();
    const updateSpy = vi.spyOn(bridgeApi, "updateChatModel");
    await click(pill);
    const opus = [...aside.querySelectorAll("button")].find(button => button.textContent?.includes("Opus"))!;
    await click(opus);
    const asideId = updateSpy.mock.calls[0]?.[0];
    expect(updateSpy).toHaveBeenCalledWith(asideId, "claude", "opus");
    expect(asideId).not.toBe("session-1");
  });

  // Contract: the /btw side chat. The command is Bridge's, not a turn for the
  // open chat: the question opens beside this conversation with its context,
  // the chat underneath keeps its selection, its forest, and its composer —
  // and the turn is delivered to the aside session, never to the parent.
  it("opens /btw as a side chat that reads the parent and never writes to it", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const titleBefore = container.querySelector("h1")!.textContent;
    const createSpy = vi.spyOn(bridgeApi, "createAsideChat");
    const submitSpy = vi.spyOn(bridgeApi, "submitInput");
    const type = async (text: string) => {
      const box = composer()!;
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
        setter.call(box, text);
        box.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
      await settle(4);
    };
    await type("/btw is the plan sound?");
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]');
    expect(aside).not.toBeNull();
    expect(aside!.textContent).toContain("is the plan sound?");
    // The parent chat never moved and its composer is clean.
    expect(container.querySelector("h1")!.textContent).toBe(titleBefore);
    expect(composer()!.value).toBe("");
    // The turn went to the aside session, never to the consulted chat.
    const sourceId = createSpy.mock.calls[0]?.[0];
    expect(sourceId).toBeTruthy();
    expect(submitSpy).toHaveBeenCalled();
    for (const call of submitSpy.mock.calls) expect(call[0]).not.toBe(sourceId);
  });

  it("treats /side as the same side-chat command", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const box = composer()!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "/side what did we pick?");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    await settle(4);
    expect(document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')).not.toBeNull();
    expect(composer()!.value).toBe("");
  });

  it("answers a bare /btw with guidance instead of opening an empty aside", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const box = composer()!;
    // A refused ask costs nothing: the image stays on the composer with the
    // draft, because it was never handed to an aside.
    const file = new File(["fake-image-bytes"], "refused.png", { type: "image/png" });
    const pasteEvent = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(pasteEvent, "clipboardData", {
      value: { items: [{ kind: "file", type: "image/png", getAsFile: () => file }] },
    });
    await act(async () => { box.dispatchEvent(pasteEvent); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "/btw");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    await settle(4);
    expect(document.body.querySelector('div[role="dialog"][aria-label^="Aside"]')).toBeNull();
    expect(container.textContent).toContain("Ask a side question");
    // The draft and its attachment survive the refusal for the retry.
    expect(box.value).toBe("/btw");
    expect(container.querySelectorAll("img").length).toBeGreaterThan(0);
  });

  // Contract: selection opens a side chat. Selecting prose in the transcript
  // raises an "Ask aside" chip; clicking it opens the side chat with the
  // excerpt quoted as its first message — the parent conversation untouched.
  it("offers Ask aside on a transcript selection and quotes the excerpt", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const transcriptRow = container.querySelector<HTMLElement>("[id^='forest-entry-']");
    const selectable = transcriptRow
      ?? [...container.querySelectorAll("h1, h2, p")].find(node => (node.textContent ?? "").trim().length > 3);
    expect(selectable).toBeTruthy();
    const range = document.createRange();
    range.selectNodeContents(selectable!);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    await act(async () => { document.dispatchEvent(new Event("selectionchange")); });
    await settle(2);
    const chip = container.querySelector<HTMLButtonElement>("[data-ask-aside-chip]");
    expect(chip).not.toBeNull();
    await click(chip!);
    await settle(4);
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label^="Aside with"]')!;
    expect(aside).not.toBeNull();
    const expectedQuote = (selectable!.textContent ?? "").trim();
    expect(aside.textContent).toContain(expectedQuote);
    // The selection is spent and the chip gone.
    expect(window.getSelection()!.isCollapsed).toBe(true);
    expect(container.querySelector("[data-ask-aside-chip]")).toBeNull();
  });

  // The chip speaks only for its own transcript: the parent conversation stays
  // mounted under an aside panel, and the chrome carries selectable text of
  // its own, so a selection anchored outside the transcript must not raise it.
  it("does not offer Ask aside for selections outside the transcript", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const title = [...container.querySelectorAll("h1, h2")].find(node => (node.textContent ?? "").trim().length > 3);
    expect(title).toBeTruthy();
    const range = document.createRange();
    range.selectNodeContents(title!);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    await act(async () => { document.dispatchEvent(new Event("selectionchange")); });
    await settle(2);
    expect(container.querySelector("[data-ask-aside-chip]")).toBeNull();
  });

  // An image on the composer is not stranded by the side-chat command: the
  // attachments ride along as the aside's first-message attachments — the same
  // delivery the aside's own composer uses — and the parent composer is left
  // clean.
  it("carries composer attachments with the /btw side chat's first message", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    const box = composer()!;
    const file = new File(["fake-image-bytes"], "side.png", { type: "image/png" });
    const pasteEvent = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(pasteEvent, "clipboardData", {
      value: { items: [{ kind: "file", type: "image/png", getAsFile: () => file }] },
    });
    await act(async () => { box.dispatchEvent(pasteEvent); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
    const submitSpy = vi.spyOn(bridgeApi, "submitInput");
    const createSpy = vi.spyOn(bridgeApi, "createAsideChat");
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "/btw what does this screenshot break?");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    await settle(6);
    const aside = document.body.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')!;
    expect(aside).not.toBeNull();
    expect(aside.textContent).toContain("what does this screenshot break?");
    const sourceId = createSpy.mock.calls[0]?.[0];
    const carried = submitSpy.mock.calls.find(([id, , attachments]) => id !== sourceId && attachments && attachments.length === 1);
    expect(carried).toBeTruthy();
    // The parent composer is clean: every rendered image lives inside the
    // aside panel (its pending bubble), none on the chat underneath.
    const outside = [...container.querySelectorAll("img")].filter(img => !aside.contains(img));
    expect(outside).toHaveLength(0);
  });

  it("keeps AppTitleBar unchanged on every other view", async () => {
    await mountApp();
    await click(container.querySelector<HTMLButtonElement>('button[title^="Open settings"]')!);
    expect(appTitleBar()).not.toBeNull();
    expect(container.querySelector('button[aria-label="Toggle dock"]')).toBeNull();
  });

  // #457 / #458: the screens stay in the tree, but nothing in the mounted
  // shell — rail, title bar, session chrome, or the keymap/menu table —
  // may offer a way into them. Sidebar-only tests would miss a later
  // title-bar, menu, or chord entry point.
  it("exposes Agent Fleet while keeping the Work board out of navigation", async () => {
    await mountApp();

    expect(container.querySelector('button[aria-label="Agent Fleet"]')).not.toBeNull();
    const hiddenNav = /^Work board$/;
    const namedControls = (root: ParentNode) =>
      [...root.querySelectorAll<HTMLElement>("button, [role='menuitem'], [role='link'], a")]
        .filter(node => hiddenNav.test((node.getAttribute("aria-label") ?? node.textContent ?? "").trim()));

    expect(namedControls(container), "welcome chrome").toEqual([]);
    expect(container.querySelector("h1")?.textContent).not.toMatch(hiddenNav);

    await openWorkspaceSession("4 files");
    expect(namedControls(container), "session chrome").toEqual([]);
    expect(container.querySelector("h1")?.textContent).not.toMatch(hiddenNav);

    // The sheet and the native menu both read this table; a new chord or
    // menu item for either screen has to land here first.
    expect(SHORTCUTS.some(shortcut => /mission|work-board|workboard/i.test(shortcut.id))).toBe(false);
    expect(SHORTCUTS.some(shortcut => /Agent Fleet|Work board/i.test(shortcut.label))).toBe(false);
  });

  it("forks a message into a new session and switches to it; rewind asks first", async () => {
    await mountApp();
    // Open the demo orchestrator; its transcript carries entry-bearing
    // messages, so the hover actions exist in the DOM even before a hover
    // reveals them.
    await openWorkspaceSession("4");
    // The transcript projects from the fetched forest; let the effects land.
    await settle(8);
    // Earlier tests mutate the shared mock (extra orchestrators, fresh
    // forests), so find the chat whose transcript carries entry-bearing
    // messages instead of assuming demo-1 is the first open.
    let forkButton = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "Fork from here");
    for (const row of chatRows()) {
      if (forkButton) break;
      await click(row);
      await settle(4);
      forkButton = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "Fork from here");
    }
    expect(forkButton).toBeTruthy();
    await click(forkButton!);
    expect(container.textContent).toContain("New branch of Orchestrator from this message");
    await click([...container.querySelectorAll("button")].find(button => button.textContent?.includes("Create fork"))!);
    // The fork is created and the app switches to it: the conversation header
    // belongs to the forked session now.
    expect(container.textContent).toContain("Fork of Orchestrator");
    const rewind = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === "Rewind to here")!;
    expect(rewind).toBeTruthy();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await click(rewind);
    expect(confirm).toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("renders a pasted session reference as a chip and pulls it into the draft", async () => {
    await mountApp();
    await openWorkspaceSession("4");
    await settle(6);
    const textarea = composer()!;
    await act(async () => {
      const nativeSet = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      nativeSet.call(textarea, "compare with @session:session-1");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await settle(8);
    expect(container.textContent).toContain("Orchestrator");
    const pull = [...container.querySelectorAll("button")].find(button => button.getAttribute("aria-label")?.startsWith("Pull Orchestrator"));
    expect(pull).toBeTruthy();
    await click(pull!);
    expect(composer()!.value).toContain("[session Orchestrator — checkpoint present]");
  });
});
