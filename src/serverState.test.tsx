// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { WorkBoard } from "./protocol/generated/protocol";
import { useBridgeServerState } from "./serverState";

const { readBoard } = vi.hoisted(() => ({ readBoard: vi.fn() }));
vi.mock("./api", () => ({ bridgeApi: {
  health: async () => ({}), modelSetup: async () => ({}), workBoard: readBoard,
} }));
let client: QueryClient;
let root: Root;
let host: HTMLDivElement;
let state: ReturnType<typeof useBridgeServerState>;
let response: WorkBoard;
function Observer() {
  state = useBridgeServerState();
  return <div>{state.workBoard?.suggestions.state}:{state.workBoard?.tasks.map(task => task.title).join(",")}</div>;
}
async function tick(ms: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
}
beforeEach(async () => {
  vi.useFakeTimers();
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity, staleTime: 30_000, refetchOnWindowFocus: false } } });
  response = { suggestions: { state: "running" }, tasks: [] } as unknown as WorkBoard;
  readBoard.mockReset().mockImplementation(async () => response);
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
  await act(async () => root.render(<QueryClientProvider client={client}><Observer /></QueryClientProvider>));
});
afterEach(() => {
  act(() => root.unmount()); client.clear(); host.remove(); vi.useRealTimers();
});

it("leaves the unopened idle board unread", async () => {
  await tick(10_000);
  expect(readBoard).not.toHaveBeenCalled();
});

it.each(["ready", "degraded"] as const)("polls the async briefing until %s and then stops", async terminal => {
  await act(async () => { await state.refetchWorkBoard(); });
  await tick(10);
  expect(host.textContent).toContain("running");
  const before = readBoard.mock.calls.length;
  await tick(2_010);
  expect(readBoard.mock.calls.length).toBeGreaterThan(before);
  response = { suggestions: { state: terminal }, tasks: [{ title: "New integration activity" }] } as unknown as WorkBoard;
  await tick(2_010);
  await tick(10);
  expect(host.textContent).toContain(`${terminal}:New integration activity`);
  const settled = readBoard.mock.calls.length;
  await tick(10_000);
  expect(readBoard).toHaveBeenCalledTimes(settled);
});

it("keeps following a running briefing after a transient read failure", async () => {
  await act(async () => { await state.refetchWorkBoard(); });
  await tick(10);
  readBoard.mockRejectedValueOnce(new Error("temporary read failure"));
  await tick(2_010);
  expect(state.workBoardQueryError?.message).toBe("temporary read failure");
  response = { suggestions: { state: "ready" }, tasks: [] } as unknown as WorkBoard;
  await tick(2_010);
  await tick(10);
  expect(state.workBoardQueryError).toBeNull();
  expect(host.textContent).toContain("ready");
});

it("cleans up polling when the observer unmounts", async () => {
  await act(async () => { await state.refetchWorkBoard(); });
  await tick(10);
  act(() => root.render(<div />));
  const before = readBoard.mock.calls.length;
  await tick(10_000);
  expect(readBoard).toHaveBeenCalledTimes(before);
});

it("follows an accepted receipt even when its first read fails over a cached ready board", async () => {
  response = { suggestions: { state: "ready" }, latestRun: { id: "old", status: "succeeded" }, tasks: [] } as unknown as WorkBoard;
  await act(async () => { await state.refetchWorkBoard(); });
  await tick(10);
  readBoard.mockImplementation(async () => { throw new Error("first read failed"); });
  await act(async () => {
    state.followWorkBriefing("new");
    await state.refetchWorkBoard();
  });
  await tick(10);
  expect(state.workBoardQueryError?.message).toBe("first read failed");
  response = { suggestions: { state: "running" }, latestRun: { id: "new", status: "running" }, tasks: [] } as unknown as WorkBoard;
  readBoard.mockImplementation(async () => response);
  await tick(2_010);
  await tick(10);
  expect(host.textContent).toContain("running");
  response = { suggestions: { state: "ready" }, latestRun: { id: "new", status: "succeeded" }, tasks: [{ title: "Fresh activity" }] } as unknown as WorkBoard;
  await tick(2_010);
  await tick(10);
  expect(host.textContent).toContain("ready:Fresh activity");
  const settled = readBoard.mock.calls.length;
  await tick(10_000);
  expect(readBoard).toHaveBeenCalledTimes(settled);
});
