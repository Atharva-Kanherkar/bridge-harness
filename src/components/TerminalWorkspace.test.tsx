// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TerminalWorkspace } from "./TerminalWorkspace";
import { bridgeApi } from "../api";
import type { TerminalRecord } from "../terminal/types";
import { addTab, emptyLayout, leafIds } from "../terminal/layout";

vi.mock("./TerminalSurface", () => ({ TerminalSurface: ({ record, focused }: { record: TerminalRecord; focused: boolean }) => <div data-terminal={record.terminalId} data-focused={focused} /> }));
vi.mock("../api", () => ({ bridgeApi: { terminalWorkspace: vi.fn(), terminalSnapshot: vi.fn(), createTerminal: vi.fn(), saveTerminalLayout: vi.fn(), closeTerminal: vi.fn(), renameTerminal: vi.fn() } }));
const api = vi.mocked(bridgeApi);
const record = (id: string, overrides: Partial<TerminalRecord> = {}): TerminalRecord => ({ workspaceId: "w", terminalId: id, generation: id, title: id, cwd: "/tmp/checkout", rows: 24, cols: 80, createdAt: "now", status: "running", ...overrides });
let host: HTMLDivElement, root: Root;
const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(b => (b.getAttribute("aria-label") ?? b.textContent ?? "").startsWith(label))!;
const click = async (label: string) => { await act(async () => { button(label).click(); }); };
const mount = async () => { await act(async () => { root.render(<TerminalWorkspace workspaceId="w" branch="main" />); }); };
beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  api.terminalWorkspace.mockResolvedValue({ terminals: [record("a")], layout: addTab(emptyLayout(), "a", "a") });
  api.terminalSnapshot.mockImplementation(async (_workspace, id) => ({ record: record(id, { cwd: "/tmp/observed" }), sequence: 0, ansi: "" }));
  api.createTerminal.mockImplementation(async params => record(params.terminalId, { agentId: params.agentId, generation: "new" }));
  api.saveTerminalLayout.mockResolvedValue(undefined); api.closeTerminal.mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
it("restores a pane without launching a process and creates distinct nested splits", async () => {
  await mount(); expect(api.createTerminal).not.toHaveBeenCalled();
  await click("Split right"); await click("Split down");
  expect(host.querySelectorAll("[data-terminal]")).toHaveLength(3);
  expect(new Set([...host.querySelectorAll("[data-terminal]")].map(e => e.getAttribute("data-terminal"))).size).toBe(3);
  expect(api.createTerminal).toHaveBeenCalledTimes(2);
  expect(api.createTerminal.mock.calls[0][0].cwd).toBe("/tmp/observed");
  expect(host.querySelectorAll('[role="separator"]')).toHaveLength(2);
});
it("moves a split to its own tab and maximizes without spawning again", async () => {
  await mount(); await click("Split right");
  await click("Maximize pane"); expect(host.querySelectorAll("[data-terminal]")).toHaveLength(1);
  await click("Restore panes"); expect(host.querySelectorAll("[data-terminal]")).toHaveLength(2);
  await click("Move focused pane to a new tab");
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(2);
  expect(api.createTerminal).toHaveBeenCalledTimes(1);
});
it("releases terminal focus before moving its input out of the DOM", async () => {
  await mount(); await click("Split right");
  const terminal = host.querySelector<HTMLElement>("[data-terminal=a]")!;
  terminal.classList.add("xterm");
  const input = document.createElement("textarea");
  terminal.append(input);
  let blurredBeforeRemoval = false;
  input.addEventListener("blur", () => { blurredBeforeRemoval = input.isConnected; });
  await act(async () => input.focus());
  expect(document.activeElement).toBe(input);
  await click("Move focused pane to a new tab");
  expect(blurredBeforeRemoval).toBe(true);
  expect(document.activeElement).not.toBe(input);
});
it("retains a pane when close fails and closes only the addressed terminal on success", async () => {
  await mount(); api.closeTerminal.mockRejectedValueOnce(new Error("host disconnected"));
  await click("Close a"); expect(host.querySelector("[data-terminal=a]")).not.toBeNull();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("host disconnected");
  await click("Close a"); expect(host.querySelector("[data-terminal=a]")).toBeNull();
  expect(api.closeTerminal).toHaveBeenLastCalledWith("w", "a");
});
it("starts an ended CLI only on explicit Restart", async () => {
  api.terminalWorkspace.mockResolvedValue({ terminals: [record("a", { status: "exited", agentId: "codex" })], layout: null });
  await mount(); expect(api.createTerminal).not.toHaveBeenCalled();
  await click("Restart"); expect(api.createTerminal).toHaveBeenCalledWith({ workspaceId: "w", terminalId: "a", restart: true });
});
it("launches an installed agent CLI with a structured agent identity", async () => {
  await mount(); await click("New Agent"); await click("Claude Code");
  expect(api.createTerminal.mock.calls[0][0]).toMatchObject({ workspaceId: "w", agentId: "claude", restart: false });
});
it("flushes the complete split tree when navigating away", async () => {
  await mount(); await click("Split down");
  await act(async () => root.render(null));
  expect(api.saveTerminalLayout).toHaveBeenCalled();
  const saved = api.saveTerminalLayout.mock.calls.at(-1)!;
  expect(saved[0]).toBe("w");
  expect(leafIds((saved[1] as ReturnType<typeof emptyLayout>).tabs[0].root)).toHaveLength(2);
});
const focusedTerminal = () => host.querySelector('[data-terminal][data-focused="true"]')?.getAttribute("data-terminal");
const dragEvent = (type: string, id: string) => {
  const event = new Event(type, { bubbles: true, cancelable: true });
  const payload = JSON.stringify({ workspaceId: "w", id });
  Object.defineProperty(event, "dataTransfer", { value: { types: ["application/x-bridge-terminal-pane"], getData: () => payload, dropEffect: "none" } });
  return event;
};
it("splits a new agent into the focused pane of the current tab", async () => {
  await mount(); await click("New Agent"); await click("Claude Code");
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(1);
  expect(host.querySelectorAll("[data-terminal]")).toHaveLength(2);
  expect(focusedTerminal()).toBe(api.createTerminal.mock.calls[0][0].terminalId);
  expect(api.createTerminal.mock.calls[0][0].cwd).toBeUndefined();
});
it("splits a new shell into the focused pane and alternates direction when unmeasurable", async () => {
  await mount(); await click("New Terminal");
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(1);
  expect(host.querySelectorAll("[data-terminal]")).toHaveLength(2);
  expect(api.createTerminal.mock.calls[0][0].cwd).toBe("/tmp/observed");
  expect(host.querySelector('[role="separator"]')?.getAttribute("aria-orientation")).toBe("vertical");
  await click("New Terminal");
  expect([...host.querySelectorAll('[role="separator"]')].map(s => s.getAttribute("aria-orientation"))).toEqual(["vertical", "horizontal"]);
});
it("opens a separate tab from the New tab menu item", async () => {
  await mount(); await click("New Agent"); await click("New tab");
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(2);
  expect(host.querySelectorAll("[data-terminal]")).toHaveLength(1);
});
it("creates the first tab when the layout is empty", async () => {
  api.terminalWorkspace.mockResolvedValue({ terminals: [], layout: null });
  await mount(); expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
  await click("New Terminal");
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(1);
  expect(host.querySelectorAll("[data-terminal]")).toHaveLength(1);
});
it("appends a pane dropped on the empty panel background beside the last leaf", async () => {
  await mount(); await click("Split right");
  const created = api.createTerminal.mock.calls[0][0].terminalId;
  const panel = host.querySelector<HTMLElement>("[data-terminal-panel]")!;
  await act(async () => { panel.dispatchEvent(dragEvent("dragover", "a")); });
  expect(panel.textContent).toContain("Drop to add beside the last pane");
  await act(async () => { panel.dispatchEvent(dragEvent("drop", "a")); });
  expect([...host.querySelectorAll("[data-terminal]")].map(e => e.getAttribute("data-terminal"))).toEqual([created, "a"]);
  expect(host.querySelectorAll('[role="tab"]')).toHaveLength(1);
  expect(api.createTerminal).toHaveBeenCalledTimes(1);
});
it("ignores drops that land on a pane rather than the background", async () => {
  await mount(); await click("Split right");
  const before = [...host.querySelectorAll("[data-terminal]")].map(e => e.getAttribute("data-terminal"));
  await act(async () => { host.querySelector("[data-terminal]")!.dispatchEvent(dragEvent("drop", "a")); });
  expect([...host.querySelectorAll("[data-terminal]")].map(e => e.getAttribute("data-terminal"))).toEqual(before);
});
it("labels agent tabs and panes with the display name instead of the raw id", async () => {
  api.terminalWorkspace.mockResolvedValue({ terminals: [record("x", { agentId: "claude", title: "claude" })], layout: null });
  await mount();
  expect(host.querySelector('[role="tab"]')?.textContent).toBe("Claude Code");
  expect(host.querySelector('[aria-label="Pane Claude Code"]')).not.toBeNull();
});
