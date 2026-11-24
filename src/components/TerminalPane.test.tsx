// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { TerminalChunk, TerminalExit } from "../types";
import type { TerminalRecord } from "../terminal/types";
import { TerminalPane, type TerminalActivity } from "./TerminalPane";
import { requestTerminalRun } from "../terminal/runQueue";

// Contract: testing/feat-dock-terminal.md §3 and testing/fix-terminal-sweep.md
// §6. Each shell renders the shared TerminalSurface, mocked here: its engine
// has its own suite, and the pane's wiring is the subject.

const written = new Map<string, string[]>();
let terminalSeq = 0;
let bracketedPaste = false;
let mode = "mock";
const focused: string[] = [];

vi.mock("./TerminalSurface", async () => {
  const { useEffect } = await import("react");
  return {
    TerminalSurface: ({ record, focused: wantsFocus, onTerminal }: { record: TerminalRecord; focused: boolean; onTerminal?: (term: unknown) => void }) => {
      useEffect(() => {
        const id = `term-${terminalSeq++}`;
        onTerminal?.({
          id,
          get modes() { return { bracketedPasteMode: bracketedPaste }; },
          focus() { focused.push(id); },
          write(data: string) { written.set(id, [...(written.get(id) ?? []), data]); },
        });
        return () => onTerminal?.(null);
        // eslint-disable-next-line react-hooks/exhaustive-deps
      }, [record.generation]);
      return <div data-surface={record.terminalId} data-generation={record.generation} data-focused={wantsFocus} />;
    },
  };
});

const record = (terminalId: string, overrides: Partial<TerminalRecord> = {}): TerminalRecord => ({ workspaceId: ws, terminalId, generation: `${terminalId}-g${++generationSeq}`, title: "Shell", cwd: "/tmp", agentId: null, status: "running", rows: 24, cols: 80, createdAt: "now", exitCode: null, historyTruncated: false, ...overrides });
let generationSeq = 0;

vi.mock("../api", () => ({
  backendMode: () => mode,
  bridgeApi: {
    createTerminal: vi.fn(),
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

const opened = () => api.createTerminal.mock.calls.map(call => call[0].terminalId);
const tabButtons = () => [...container.querySelectorAll<HTMLButtonElement>("button[title*='double-click to rename']")];
const hostFor = (terminalId: string) => container.querySelector<HTMLElement>(`[data-shell-host="${terminalId}"]`);

const emitChunk = async (terminalId: string, data: string) => {
  await act(async () => {
    chunkHandler?.({ sessionId: ws, terminalId, data });
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  ws = `w${++wsSeq}`;
  vi.clearAllMocks();
  written.clear();
  focused.length = 0;
  bracketedPaste = false;
  mode = "mock";
  chunkHandler = undefined;
  exitHandler = undefined;
  api.listTerminals.mockResolvedValue([]);
  api.createTerminal.mockImplementation(async params => record(params.terminalId));
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
    expect(api.createTerminal).toHaveBeenCalledWith({ workspaceId: ws, terminalId: expect.stringMatching(/^t\d+$/), restart: true });
    const first = api.createTerminal.mock.calls[0][0].terminalId;
    expect(tabButtons()).toHaveLength(1);

    await click(container.querySelector('button[aria-label="New shell"]')!);
    expect(tabButtons()).toHaveLength(2);
    const second = api.createTerminal.mock.calls[1][0].terminalId;
    expect(second).not.toBe(first);
    expect(hostFor(second)!.className).not.toContain("hidden");
    expect(hostFor(first)!.className).toContain("hidden");
  });

  it("reattaches to live shells instead of respawning", async () => {
    api.listTerminals.mockResolvedValue(["t4", "t7"]);
    await mount();
    expect(tabButtons()).toHaveLength(2);
    expect(api.createTerminal).toHaveBeenCalledTimes(2);
    expect(opened()).toEqual(["t4", "t7"]);

    // The counter learned from the live ids: "+" creates, never re-activates.
    await click(container.querySelector('button[aria-label="New shell"]')!);
    expect(opened().at(-1)).toBe("t8");
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

  it("leaves bytes to each surface's frames and keeps hosts mounted across switches", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    const hostT1 = hostFor("t1")!;
    expect(hostT1.querySelector('[data-surface="t1"]')).not.toBeNull();
    await emitChunk("t2", "from-two");
    // The surface draws from sequenced frames; a second write from the
    // session-output stream is what used to print every byte twice.
    expect([...written.values()].flat()).toEqual([]);

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
    expect(api.createTerminal).toHaveBeenCalledTimes(1);
    expect(api.writeTerminal).not.toHaveBeenCalled();
  });

  it("states the worktree path and the scrollback bound", async () => {
    await mount();
    expect(container.textContent).toContain(`/tmp/bridge/${ws}`);
    expect(container.textContent).toContain("5,000 lines scrollback");
  });

  it("mounts the shared surface with the record the runtime returned", async () => {
    let resolve: (value: TerminalRecord) => void = () => undefined;
    api.createTerminal.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
    await mount();
    expect(container.querySelector("[data-surface]")).toBeNull();
    expect(container.textContent).toContain("Starting shell");
    await act(async () => { resolve(record("t1", { generation: "live-1" })); });
    expect(container.querySelector('[data-surface="t1"]')!.getAttribute("data-generation")).toBe("live-1");
  });

  it("restarts an ended shell on request", async () => {
    api.listTerminals.mockResolvedValue(["t1"]);
    await mount();
    await act(async () => { exitHandler?.({ sessionId: ws, terminalId: "t1" }); });
    expect(container.textContent).toContain("Shell ended");
    const before = container.querySelector('[data-surface="t1"]')!.getAttribute("data-generation");
    await click([...container.querySelectorAll("button")].find(button => button.textContent === "Restart")!);
    expect(api.createTerminal).toHaveBeenLastCalledWith({ workspaceId: ws, terminalId: "t1", restart: true });
    expect(container.querySelector('[data-shell-mark="exited"]')).toBeNull();
    expect(container.querySelector('[data-surface="t1"]')!.getAttribute("data-generation")).not.toBe(before);
    expect(container.textContent).not.toContain("Shell ended");
  });

  it("shows why a shell could not start and lets it try again", async () => {
    api.createTerminal.mockRejectedValueOnce(new Error("Node.js is required for persistent terminals."));
    await mount();
    expect(container.querySelector('[role="alert"]')!.textContent).toContain("Node.js is required");
    await click([...container.querySelectorAll("button")].find(button => button.textContent === "Try again")!);
    expect(container.querySelector('[data-surface="t1"]')).not.toBeNull();
  });

  it("labels a terminal Agent Fleet started with its title", async () => {
    api.listTerminals.mockResolvedValue(["9b2f6c1e-agent"]);
    api.createTerminal.mockImplementation(async params => record(params.terminalId, { title: "Dev server" }));
    await mount();
    expect(tabButtons()[0].textContent).toBe("Dev server");
  });

  it("focuses a shell the user picks, only while the pane is visible", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    expect(container.querySelector('[data-surface="t1"]')!.getAttribute("data-focused")).toBe("false");
    await click(tabButtons()[1]);
    expect(container.querySelector('[data-surface="t2"]')!.getAttribute("data-focused")).toBe("true");
    await mount({ visible: false });
    expect(container.querySelector('[data-surface="t2"]')!.getAttribute("data-focused")).toBe("false");
  });
});

// Contract: testing/feat-run-in-terminal.md §Integration (TerminalPane).
describe("TerminalPane runs from the chat", () => {
  /** Let React render between awaits until the run settles: a run that opens
   *  a shell needs that shell's host to mount, which never happens while a
   *  single act scope is still awaiting the run. */
  async function settled<T>(run: Promise<T>): Promise<T> {
    let done = false;
    void run.then(() => { done = true; }, () => { done = true; });
    for (let turn = 0; turn < 100 && !done; turn += 1) {
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
    }
    return run;
  }
  const writesTo = (terminalId: string) => api.writeTerminal.mock.calls.filter(call => call[1] === terminalId).map(call => call[2]);

  it("takes a run queued before it mounted and types it into the shell in one write", async () => {
    const result = requestTerminalRun(ws, "bun install\nbun test");
    await mount();
    await settled(result);
    expect(opened()).toEqual(["t1"]);
    expect(writesTo("t1")).toEqual(["bun install\rbun test\r"]);
    expect(api.createTerminal.mock.invocationCallOrder[0]).toBeLessThan(api.writeTerminal.mock.invocationCallOrder[0]);
    await expect(result).resolves.toEqual({ shell: "shell 1" });
    expect(focused).toHaveLength(1);
  });

  it("still opens a shell and takes runs when the shell list fails", async () => {
    api.listTerminals.mockRejectedValue(new Error("list failed"));
    await mount();
    expect(opened()).toEqual(["t1"]);
    const result = await settled(requestTerminalRun(ws, "claude mcp add demo"));
    expect(writesTo("t1")).toEqual(["claude mcp add demo\r"]);
    expect(result).toEqual({ shell: "shell 1" });
  });

  it("runs in the active shell, not the first one", async () => {
    api.listTerminals.mockResolvedValue(["t1", "t2"]);
    await mount();
    await click(tabButtons()[1]);
    await settled(requestTerminalRun(ws, "pwd"));
    expect(writesTo("t2")).toEqual(["pwd\r"]);
    expect(writesTo("t1")).toEqual([]);
  });

  it("opens a new shell when the active one has exited", async () => {
    api.listTerminals.mockResolvedValue(["t1"]);
    await mount();
    await act(async () => { exitHandler?.({ sessionId: ws, terminalId: "t1" }); });
    const result = await settled(requestTerminalRun(ws, "ls"));
    expect(opened().at(-1)).toBe("t2");
    expect(writesTo("t2")).toEqual(["ls\r"]);
    expect(writesTo("t1")).toEqual([]);
    expect(result).toEqual({ shell: "shell 2" });
    expect(tabButtons()).toHaveLength(2);
    expect(hostFor("t2")!.className).not.toContain("hidden");
  });

  it("pastes as one edit when the shell asked for bracketed paste", async () => {
    bracketedPaste = true;
    api.listTerminals.mockResolvedValue(["t1"]);
    await mount();
    await settled(requestTerminalRun(ws, "cd app\nmake"));
    expect(writesTo("t1")).toEqual(["\x1b[200~cd app\rmake\x1b[201~\r"]);
  });

  it("waits for a shell it just spawned to draw its prompt", async () => {
    mode = "desktop";
    await mount();
    const result = requestTerminalRun(ws, "echo hi");
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 120)); });
    expect(writesTo("t1")).toEqual([]);
    await emitChunk("t1", "~/app $ ");
    await settled(result);
    expect(writesTo("t1")).toEqual(["echo hi\r"]);
  });

  it("does not wait on a shell that was already alive", async () => {
    mode = "desktop";
    api.listTerminals.mockResolvedValue(["t1"]);
    await mount();
    const started = Date.now();
    await settled(requestTerminalRun(ws, "echo hi"));
    expect(writesTo("t1")).toEqual(["echo hi\r"]);
    expect(Date.now() - started).toBeLessThan(PROMPT_FREE_MS);
  });
});

const PROMPT_FREE_MS = 1_000;
