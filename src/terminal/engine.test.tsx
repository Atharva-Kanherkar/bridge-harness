// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalRecord } from "./types";

// Contract: testing/fix-terminal-sweep.md §4. xterm paints to a canvas jsdom
// lacks; the fake records what the engine asks of it.
const { FakeTerminal, terminals, fit, unlisten } = vi.hoisted(() => {
  type Handler = (data: string) => void;
  const terminals: FakeTerminal[] = [];
  class FakeTerminal {
    options: Record<string, unknown>;
    unicode = { activeVersion: "6" };
    cols = 80;
    rows = 24;
    opened: HTMLElement[] = [];
    resizes: [number, number][] = [];
    disposed = false;
    focusCalls = 0;
    data?: Handler;
    binary?: Handler;
    modes = { bracketedPasteMode: false };
    constructor(options: Record<string, unknown>) { this.options = options; terminals.push(this); }
    loadAddon() {}
    open(element: HTMLElement) { this.opened.push(element); }
    onData(handler: Handler) { this.data = handler; return { dispose() {} }; }
    onBinary(handler: Handler) { this.binary = handler; return { dispose() {} }; }
    attachCustomKeyEventHandler() {}
    write(_data: string, done?: () => void) { done?.(); }
    reset() {}
    resize(cols: number, rows: number) { this.resizes.push([cols, rows]); this.cols = cols; this.rows = rows; }
    focus() { this.focusCalls++; }
    blur() {}
    refresh() {}
    hasSelection() { return false; }
    dispose() { this.disposed = true; }
  }
  return { FakeTerminal, terminals, fit: { proposed: { cols: 100, rows: 30 } as { cols: number; rows: number } | undefined }, unlisten: vi.fn() };
});
type FakeTerminal = InstanceType<typeof FakeTerminal>;
vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { proposeDimensions() { return fit.proposed; } } }));
vi.mock("@xterm/addon-search", () => ({ SearchAddon: class { onDidChangeResults() { return { dispose() {} }; } } }));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class {} }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("./webgl", () => ({ attachWebgl: () => () => undefined }));
vi.mock("../api", () => ({
  bridgeApi: {
    terminalSnapshot: vi.fn(),
    onTerminalFrame: vi.fn(async () => unlisten),
    onTerminalLagged: vi.fn(async () => unlisten),
    writeTerminal: vi.fn(async () => undefined),
    resizeTerminal: vi.fn(async () => undefined),
  },
}));

import { act } from "react";
import { createRoot } from "react-dom/client";
import { bridgeApi } from "../api";
import { acquireTerminalEngine, releaseTerminalEngine, TerminalEngine } from "./engine";
import { TerminalSurface } from "../components/TerminalSurface";

const api = vi.mocked(bridgeApi);
const record = (overrides: Partial<TerminalRecord> = {}): TerminalRecord => ({ workspaceId: "w", terminalId: "t", generation: "g", title: "Shell", cwd: "/tmp", agentId: null, status: "running", rows: 24, cols: 80, createdAt: "now", exitCode: null, historyTruncated: false, ...overrides });
const sized = (element: HTMLElement, width: number, height: number) => {
  Object.defineProperty(element, "clientWidth", { value: width, configurable: true });
  Object.defineProperty(element, "clientHeight", { value: height, configurable: true });
};
const engines: TerminalEngine[] = [];
function mounted(identity = record(), size: [number, number] = [800, 400]) {
  const host = document.createElement("div");
  document.body.append(host);
  const engine = new TerminalEngine(identity);
  engines.push(engine);
  sized(engine.element, ...size);
  engine.attach(host);
  return { engine, host, term: terminals.at(-1)! };
}
const restored = (term: FakeTerminal) => vi.waitFor(() => { expect(api.terminalSnapshot).toHaveBeenCalled(); expect(term.opened.length + term.resizes.length).toBeGreaterThan(0); });

beforeEach(() => {
  vi.clearAllMocks();
  unlisten.mockClear();
  terminals.length = 0;
  fit.proposed = { cols: 100, rows: 30 };
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
  api.terminalSnapshot.mockImplementation(async () => ({ record: record(), sequence: 0, ansi: "" }));
});
afterEach(() => {
  engines.splice(0).forEach(engine => engine.dispose());
  document.body.innerHTML = "";
  Reflect.deleteProperty(document, "fonts");
  vi.unstubAllGlobals();
});

describe("TerminalEngine", () => {
  it("gives TUIs the options they need", () => {
    const { term } = mounted();
    expect(term.unicode.activeVersion).toBe("11");
    expect(term.options.convertEol).toBeFalsy();
    expect(term.options.macOptionClickForcesSelection).toBe(true);
    expect(term.options.allowProposedApi).toBe(true);
  });

  it("never fits or resizes the PTY while its host is hidden", async () => {
    const { engine, term } = mounted(record(), [0, 0]);
    await vi.waitFor(() => expect(api.terminalSnapshot).toHaveBeenCalled());
    await new Promise(resolve => setTimeout(resolve, 0));
    engine.layout();
    expect(term.opened).toHaveLength(0);
    // Only the snapshot's own size; never FitAddon's 2x1 for a hidden box.
    expect(term.resizes).toEqual([[80, 24]]);
    expect(api.resizeTerminal).not.toHaveBeenCalled();
  });

  it("opens into an unpadded parent and sizes the PTY once visible", async () => {
    const { engine, term } = mounted();
    await restored(term);
    await vi.waitFor(() => expect(term.opened).toHaveLength(1));
    expect(term.opened[0]).toBe(engine.element);
    expect(engine.element.className).not.toMatch(/\bp[xytrbl]?-/);
    await vi.waitFor(() => expect(api.resizeTerminal).toHaveBeenCalledWith("w", "t", 30, 100));
  });

  it("re-measures a cell measured while hidden instead of giving up", async () => {
    fit.proposed = undefined;
    const { term } = mounted();
    await vi.waitFor(() => expect(term.opened).toHaveLength(1));
    // The snapshot sizes it once; the same-size resize after that is how
    // xterm re-measures an invalid cell.
    await vi.waitFor(() => expect(term.resizes.filter(([cols, rows]) => cols === 80 && rows === 24)).toHaveLength(2));
    expect(api.resizeTerminal).not.toHaveBeenCalled();
  });

  it("forwards binary mouse reports as bytes", async () => {
    const { term } = mounted();
    await restored(term);
    await vi.waitFor(() => expect(term.opened).toHaveLength(1));
    term.binary!("\x1b[M \xff!");
    term.data!("q");
    await vi.waitFor(() => expect(api.writeTerminal).toHaveBeenCalledTimes(2));
    expect(api.writeTerminal.mock.calls).toEqual([["w", "t", "\x1b[M \xff!", true], ["w", "t", "q", false]]);
  });

  it("sends its own size again after another view resized the PTY", async () => {
    const { engine, term } = mounted();
    await vi.waitFor(() => expect(api.resizeTerminal).toHaveBeenCalledWith("w", "t", 30, 100));
    const frames = api.onTerminalFrame.mock.calls[0][0];
    // The dock, at another size, resized the shared PTY.
    frames({ workspaceId: "w", terminalId: "t", generation: "g", sequence: 1, data: "", rows: 20, cols: 70 });
    await vi.waitFor(() => expect(term.cols).toBe(70));
    engine.layout();
    await vi.waitFor(() => expect(api.resizeTerminal).toHaveBeenCalledTimes(2));
    expect(api.resizeTerminal).toHaveBeenLastCalledWith("w", "t", 30, 100);
    expect([term.cols, term.rows]).toEqual([100, 30]);
  });

  it("waits for the terminal font before opening", async () => {
    let loaded!: () => void;
    const load = vi.fn(() => new Promise<void>(resolve => { loaded = resolve; }));
    Object.defineProperty(document, "fonts", { value: { load }, configurable: true });
    const { term } = mounted();
    await restored(term);
    expect(load).toHaveBeenCalledWith(expect.stringContaining("Geist Mono Variable"));
    expect(term.opened).toHaveLength(0);
    loaded();
    await vi.waitFor(() => expect(term.opened).toHaveLength(1));
  });
});

describe("engine parking", () => {
  it("lets a remount of the same terminal adopt the live engine", async () => {
    const identity = record({ terminalId: "parked" });
    const first = acquireTerminalEngine(identity);
    engines.push(first);
    await vi.waitFor(() => expect(api.terminalSnapshot).toHaveBeenCalledTimes(1));
    releaseTerminalEngine(first);
    const second = acquireTerminalEngine(identity);
    expect(second).toBe(first);
    await new Promise(resolve => setTimeout(resolve, 5));
    expect(terminals[0].disposed).toBe(false);
    expect(api.terminalSnapshot).toHaveBeenCalledTimes(1);
  });

  it("disposes an engine nobody adopts", async () => {
    const engine = acquireTerminalEngine(record({ terminalId: "gone" }));
    await vi.waitFor(() => expect(api.onTerminalLagged).toHaveBeenCalled());
    await vi.waitFor(() => expect(api.terminalSnapshot).toHaveBeenCalled());
    releaseTerminalEngine(engine);
    await new Promise(resolve => setTimeout(resolve, 5));
    expect(terminals[0].disposed).toBe(true);
    expect(unlisten).toHaveBeenCalledTimes(2);
    const next = acquireTerminalEngine(record({ terminalId: "gone" }));
    engines.push(next);
    expect(next).not.toBe(engine);
  });

  it("never adopts across generations", () => {
    const old = acquireTerminalEngine(record({ terminalId: "r", generation: "1" }));
    releaseTerminalEngine(old);
    const fresh = acquireTerminalEngine(record({ terminalId: "r", generation: "2" }));
    engines.push(fresh);
    expect(fresh).not.toBe(old);
  });
});

describe("TerminalSurface remounts", () => {
  it("keeps the live xterm when a split changes the tree above a pane", async () => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    const pane = record({ terminalId: "split-me" });
    const surface = <TerminalSurface record={pane} focused={false} onRecord={() => {}} />;
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    // A leaf, then the same leaf as the first child of a new split node: the
    // element type above it changes, so React remounts the surface.
    await act(async () => { root.render(<section>{surface}</section>); });
    await vi.waitFor(() => expect(api.terminalSnapshot).toHaveBeenCalledTimes(1));
    const xterm = terminals.at(-1);
    await act(async () => { root.render(<div className="grid"><div>{surface}</div><div role="separator" /></div>); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(terminals.at(-1)).toBe(xterm);
    expect(xterm!.disposed).toBe(false);
    expect(api.terminalSnapshot).toHaveBeenCalledTimes(1);
    expect(container.querySelector("[role=separator]")!.previousElementSibling!.contains(xterm!.opened[0] ?? container.querySelector(".h-full.w-full"))).toBe(true);
    await act(async () => root.unmount());
    await new Promise(resolve => setTimeout(resolve, 5));
    expect(xterm!.disposed).toBe(true);
  });
});
