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
    expect(source).not.toContain('paradigm === "grid" ? "Focus" : "Mission Control"');
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
    const surface = () => [...dockAside()!.querySelectorAll("*")].find(node => node.textContent === "Connect your browser once");
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
    expect(dockAside()!.textContent).toContain("Connect your browser once");
  });

  it("raises waiting_for_you onto the switcher while another pane is active", async () => {
    const waiting = {
      transportConnected: true, extensionId: "ext", extensionPath: "/ext",
      nativeHostInstalled: true, nativeHostManifestPath: "/m",
      tabs: [{ id: 1, title: "Example", domain: "example.com", url: "https://example.com", attached: true }],
      lease: { id: "lease-1", tabId: 1, domain: "example.com", permission: "read_only", grantedAt: "now", expiresAt: null },
      status: "waiting_for_you", captureActive: false, captureError: null, screenshot: null,
      screenshotRedactedRegions: 0, elements: [], viewport: null, promptInjectionSuspected: false,
      tokenAccounting: { snapshots: 0, fullSnapshots: 0, deltaSnapshots: 0, serializedBytes: 0, estimatedInputTokens: 0, screenshotCount: 0 },
      promptInjectionSignals: [], pendingApproval: null, audit: [], debugEvents: [], siteMetrics: [], remoteProvider: null,
    };
    const spy = vi.spyOn(bridgeApi, "browserBridgeState").mockResolvedValue(waiting as unknown as Awaited<ReturnType<typeof bridgeApi.browserBridgeState>>);
    await mountApp();
    await openWorkspaceSession("4 files");
    await key({ ...chord, code: "Digit4", key: "4" });
    await settle(3);
    await key({ ...chord, code: "Digit1", key: "1" });
    expect(container.querySelector('[data-testid="dock-alert-browser"]')).not.toBeNull();
    spy.mockRestore();
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

  // A worker session renders SteerComposer instead of the chat ComposerPill,
  // so anything wired only into the latter (usage health included) silently
  // disappears the moment you open a background worker.
  it("keeps usage health reachable from a worker session's steer composer, not only the chat composer", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    await click(dockToggle()!);
    await key({ ...chord, code: "Digit6", key: "6" });
    await settle(3);
    const openWorker = dockAside()!.querySelector<HTMLButtonElement>('button[aria-label="Open worker Implementation · strong"]')!;
    await click(openWorker);

    expect(container.querySelector("h1")!.textContent).toContain("Implementation");
    expect(container.textContent).toContain("This is a background worker");
    expect(container.querySelector('[aria-label^="Open usage health details"]')).not.toBeNull();
  });

  it("keeps usage health reachable on the pre-session Welcome view, where there is no composer to trail", async () => {
    await mountApp();
    // No session is selected yet: the pre-session Welcome screen keeps the
    // title bar, and usage health must remain reachable from it — the move
    // into the composer relocates the trigger, it does not remove it.
    expect(container.querySelector("header")).not.toBeNull();
    expect(container.querySelector('[aria-label^="Open usage health details"]')).not.toBeNull();
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
  const appTitleBar = () => container.querySelector("header.bg-transparent");

  it("collapses to one chrome row on the session view — no second AppTitleBar strip", async () => {
    await mountApp();
    await openWorkspaceSession("4 files");
    expect(appTitleBar()).toBeNull();
    expect(container.querySelector('button[aria-label="Toggle dock"]')).not.toBeNull();
  });

  it("opens the toolbar model picker downward, into the view rather than above it", async () => {
    await mountApp();
    await openWorkspaceSession("7 files");
    const pill = [...container.querySelectorAll<HTMLButtonElement>('button[aria-label*="model:"]')][0];
    expect(pill).toBeTruthy();
    await click(pill);
    // The toolbar is the first row of an overflow-hidden <main>; an upward
    // panel there is clipped out of existence.
    const panel = container.querySelector<HTMLElement>(".u-glass-popover")!;
    expect(panel).not.toBeNull();
    expect(panel.className).toContain("top-full");
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
    const aside = container.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')!;
    expect(aside).not.toBeNull();
    expect(aside.textContent).toContain("is the plan sound?");
    // The chat underneath never moved.
    expect(container.querySelector("h1")!.textContent).toBe(titleBefore);

    // Promote: the aside becomes the active chat and the panel goes away.
    const promote = [...aside.querySelectorAll("button")].find(button => button.textContent?.includes("Open as chat"))!;
    await click(promote);
    expect(container.querySelector('div[role="dialog"][aria-label^="Aside"]')).toBeNull();
    expect(container.querySelector("h1")!.textContent).toContain("is the plan sound?");
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
    const aside = container.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Codex"]');
    expect(aside).not.toBeNull();
    expect(createSpy).toHaveBeenCalledWith(expect.any(String), "codex", "gpt-5.6-terra", expect.anything());
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
    const aside = container.querySelector<HTMLElement>('div[role="dialog"][aria-label="Aside with Claude"]')!;
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
  it("exposes no Mission Control or Work board navigation control in the shell", async () => {
    await mountApp();

    const hiddenNav = /^(Mission Control|Work board)$/;
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
    expect(SHORTCUTS.some(shortcut => /Mission Control|Work board/i.test(shortcut.label))).toBe(false);
  });
});
