// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { TerminalChunk, TerminalExit } from "../types";
import { TerminalPane, type TerminalActivity } from "./TerminalPane";

// Contract: testing/feat-dock-terminal.md §3. xterm paints to canvas, which
// jsdom lacks; the terminals are mocked and the pane's wiring is the subject.

const written = new Map<string, string[]>();
let terminalSeq = 0;

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    id = `term-${terminalSeq++}`;
    options: Record<string, unknown> = {};
    open() {}
    loadAddon() {}
    dispose() {}
    onData() { return { dispose() {} }; }
    write(data: string) {
      const log = written.get(this.id) ?? [];
      log.push(data);
      written.set(this.id, log);
    }
    writeln(data: string) {
      this.write(`${data}\n`);
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

vi.mock("../api", () => ({
  bridgeApi: {
    openTerminal: vi.fn().mockResolvedValue(undefined),
    writeTerminal: vi.fn().mockResolvedValue(undefined),
    resizeTerminal: vi.fn().mockResolvedValue(undefined),
    closeTerminal: vi.fn().mockResolvedValue(undefined),
    listTerminals: vi.fn().mockResolvedValue([]),
    onTerminal: vi.fn(),
    onTerminalExited: vi.fn(),
  },
}));

const api = vi.mocked(bridgeApi);
let chunkHandler: ((chunk: TerminalChunk) => void) | undefined;
let exitHandler: ((exit: TerminalExit) => void) | undefined;

let container: HTMLDivElement;
let root: Root;
let wsSeq = 0;
let ws = "";

async function mount(props: Partial<Parameters<typeof TerminalPane>[0]> = {}) {
  await act(async () => {
    root.render(<TerminalPane workspaceId={ws} workspacePath={`/tmp/bridge/${ws}`} {...props} />);
  });
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

const tabButtons = () => [...container.querySelectorAll<HTMLButtonElement>("button[title*='double-click to rename']")];
const hostFor = (terminalId: string) => container.querySelector<HTMLElement>(`[data-shell-host="${terminalId}"]`);

const emitChunk = async (terminalId: string, data: string) => {
  await act(async () => {
    chunkHandler?.({ sessionId: ws, terminalId, data });
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  ws = `w${++wsSeq}`;
  vi.clearAllMocks();
  written.clear();
  chunkHandler = undefined;
  exitHandler = undefined;
  api.listTerminals.mockResolvedValue([]);
  api.onTerminal.mockImplementation(async handler => { chunkHandler = handler; return () => undefined; });
  api.onTerminalExited.mockImplementation(async handler => { exitHandler = handler; return () => undefined; });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("TerminalPane multi-shell", () => {
  it("creates the first shell on demand and more from the strip", async () => {
    await mount();
    expect(api.openTerminal).toHaveBeenCalledWith(ws, expect.stringMatching(/^t\d+$/));
    const first = api.openTerminal.mock.calls[0][1];
    expect(tabButtons()).toHaveLength(1);

    await click(container.querySelector('button[aria-label="New shell"]')!);
    expect(tabButtons()).toHaveLength(2);
    const second = api.openTerminal.mock.calls[1][1];
    expect(second).not.toBe(first);
    expect(hostFor(second)!.className).not.toContain("hidden");
    expect(hostFor(first)!.className).toContain("hidden");
  });

  it("reattaches to live shells instead of respawning", async () => {
    api.listTerminals.mockResolvedValue(["t4", "t7"]);
    await mount();
    expect(tabButtons()).toHaveLength(2);
    expect(api.openTerminal).toHaveBeenCalledTimes(2);
    expect(api.openTerminal).toHaveBeenCalledWith(ws, "t4");
    expect(api.openTerminal).toHaveBeenCalledWith(ws, "t7");

    // The counter learned from the live ids: "+" creates, never re-activates.
    await click(container.querySelector('button[aria-label="New shell"]')!);
    expect(api.openTerminal).toHaveBeenLastCalledWith(ws, "t8");
    expect(tabButtons()).toHaveLength(3);
  });

  it("clears an exited mark when the reopened shell speaks", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    await click(tabButtons()[0]);
    await act(async () => {
      exitHandler?.({ sessionId: ws, terminalId: "t2" });
    });
    expect(container.querySelector('[data-shell-mark="exited"]')).not.toBeNull();

    await emitChunk("t2", "$ ");
    expect(container.querySelector('[data-shell-mark="exited"]')).toBeNull();
    expect(container.querySelector('[data-shell-mark="output"]')).not.toBeNull();
  });

  it("routes output to its shell only and keeps hosts mounted across switches", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    const hostT1 = hostFor("t1")!;
    await emitChunk("t2", "from-two");
    const logs = [...written.values()].flat();
    expect(logs.filter(entry => entry === "from-two")).toHaveLength(1);

    await click(tabButtons()[1]);
    expect(hostFor("t1")).toBe(hostT1);
    expect(hostFor("t1")!.className).toContain("hidden");
    expect(hostFor("t2")!.className).not.toContain("hidden");
  });

  it("closes a shell only through its explicit control", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    await click(tabButtons()[0]);
    await click(container.querySelector('button[aria-label="Close shell 1"]')!);
    expect(api.closeTerminal).toHaveBeenCalledWith(ws, "t1");
    expect(api.closeTerminal).toHaveBeenCalledTimes(1);
    expect(tabButtons()).toHaveLength(1);
  });

  it("marks background output and clears the mark on activation", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    await click(tabButtons()[0]);
    await emitChunk("t2", "psst");
    expect(container.querySelector('[data-shell-mark="output"]')).not.toBeNull();
    await click(tabButtons()[1]);
    expect(container.querySelector('[data-shell-mark="output"]')).toBeNull();
  });

  it("shows an exit and reports activity upward", async () => {
    const seen: TerminalActivity[] = [];
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount({ onActivity: activity => seen.push(activity) });
    expect(seen.at(-1)).toEqual({ running: 2, attention: false });

    await act(async () => {
      exitHandler?.({ sessionId: ws, terminalId: "t2" });
    });
    expect(container.querySelector('[data-shell-mark="exited"]')).not.toBeNull();
    expect(seen.at(-1)).toEqual({ running: 1, attention: true });
  });

  it("renames a tab as a label without any api call", async () => {
    api.listTerminals.mockResolvedValue(["t1"]);
    await mount();
    const tab = tabButtons()[0];
    await act(async () => {
      tab.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    const input = container.querySelector<HTMLInputElement>('input[aria-label="Rename t1"]')!;
    await act(async () => {
      input.value = "dev server";
      input.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });
    expect(tabButtons()[0].textContent).toBe("dev server");
    expect(api.openTerminal).toHaveBeenCalledTimes(1);
    expect(api.writeTerminal).not.toHaveBeenCalled();
  });

  it("states the worktree path and the scrollback bound", async () => {
    await mount();
    expect(container.textContent).toContain(`/tmp/bridge/${ws}`);
    expect(container.textContent).toContain("1 MB scrollback");
  });
});
