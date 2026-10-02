// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import type { ArchiveChatResult, BridgeState, Session } from "./types";

let container: HTMLDivElement;
let root: Root;
let currentState: BridgeState;
const chat = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, harness: "claude", kind: "chat", label: id, title: id,
  status: "working", metricSource: "provider", restorationMode: "fresh",
  continuationFidelity: "native", ...overrides,
});

async function settle() {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
}

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => store.set(key, value),
      removeItem: (key: string) => store.delete(key), clear: () => store.clear(),
    },
  });
  currentState = { projects: [], workspaces: [], sessions: [chat("Archive me"), chat("Keep me")], events: [] };
  vi.spyOn(bridgeApi, "state").mockImplementation(async () => currentState);
  vi.spyOn(window, "confirm").mockReturnValue(true);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await act(async () => recommended.click()); await settle(); }
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const trigger = (name = "Archive me") => container.querySelector<HTMLButtonElement>(`button[aria-label="Chat actions for ${name}"]`);
async function archive(name = "Archive me") {
  expect(trigger(name)).toBeTruthy();
  await act(async () => trigger(name)!.click());
  const item = document.querySelector<HTMLButtonElement>(`[role="menuitem"][aria-label="Archive ${name}"]`);
  expect(item).toBeTruthy();
  await act(async () => item!.click());
}

it("automatically archives a working chat and confirms success before refresh finishes", async () => {
  let finish!: (result: ArchiveChatResult) => void;
  const request = vi.spyOn(bridgeApi, "archiveChat").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const stop = vi.spyOn(bridgeApi, "stopSession");
  await archive();
  expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("stop automatically"));
  expect(request).toHaveBeenCalledWith("Archive me");
  expect(trigger()).toBeTruthy();
  // Reopening the menu while shutdown is pending must not send a second call.
  await archive();
  expect(request).toHaveBeenCalledTimes(1);
  expect(window.confirm).toHaveBeenCalledTimes(1);
  let refresh!: (state: BridgeState) => void;
  vi.mocked(bridgeApi.state).mockImplementationOnce(() => new Promise(resolve => { refresh = resolve; }));
  await act(async () => finish({ archived: true, bytesFreed: 0, worktreeDetail: null }));
  await settle();
  expect(trigger()).toBeNull();
  expect(trigger("Keep me")).toBeTruthy();
  expect(container.textContent).toContain("Chat archived");
  expect(container.textContent).toContain("Settings > Archived chats");
  expect(stop).not.toHaveBeenCalled(); // One backend operation owns shutdown.
  currentState = { ...currentState, sessions: [chat("Keep me")] };
  await act(async () => refresh(currentState));
});

it("shows retained unsaved work as part of the archive confirmation", async () => {
  vi.spyOn(bridgeApi, "archiveChat").mockImplementation(async () => {
    currentState = { ...currentState, sessions: [chat("Keep me")] };
    return { archived: true, bytesFreed: 0, worktreeDetail: "uncommitted changes" };
  });
  await archive();
  expect(trigger()).toBeNull();
  expect(container.textContent).toContain("Chat archived");
  expect(container.textContent).toContain("Its worktree was kept: uncommitted changes");
});

it("keeps a failed archive visible and allows retry", async () => {
  const request = vi.spyOn(bridgeApi, "archiveChat").mockRejectedValue(new Error("Cannot stop this session"));
  await archive();
  expect(trigger()).toBeTruthy();
  expect(container.textContent).not.toContain("Chat archived");
  expect(container.textContent).toContain("Cannot stop this session");
  await archive();
  expect(request).toHaveBeenCalledTimes(2);
});

it("does nothing when the archive confirmation is cancelled", async () => {
  vi.mocked(window.confirm).mockReturnValue(false);
  const request = vi.spyOn(bridgeApi, "archiveChat");
  await archive();
  expect(request).not.toHaveBeenCalled();
  expect(trigger()).toBeTruthy();
});

it("preserves the selected conversation when archiving a background chat", async () => {
  const row = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.title.startsWith("Keep me"));
  expect(row).toBeTruthy();
  await act(async () => row!.click());
  await settle();
  vi.spyOn(bridgeApi, "archiveChat").mockImplementation(async () => {
    currentState = { ...currentState, sessions: [chat("Keep me")] };
    return { archived: true, bytesFreed: 0, worktreeDetail: null };
  });
  await archive();
  const selected = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.title.startsWith("Keep me"));
  expect(selected?.getAttribute("aria-current")).toBe("page");
});

it("leaves the selected chat surface when its archive succeeds", async () => {
  const row = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.title.startsWith("Archive me"));
  await act(async () => row!.click());
  await settle();
  expect(row!.getAttribute("aria-current")).toBe("page");
  vi.spyOn(bridgeApi, "archiveChat").mockImplementation(async () => {
    currentState = { ...currentState, sessions: [chat("Keep me")] };
    return { archived: true, bytesFreed: 0, worktreeDetail: null };
  });
  await archive();
  expect(trigger()).toBeNull();
  expect(container.querySelector('button[aria-current="page"]')).toBeNull();
  expect(container.textContent).toContain("Chat archived");
});
