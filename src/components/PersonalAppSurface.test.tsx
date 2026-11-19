// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { loadApps, resetAppsStore } from "../apps/store";
import { MOCK_PERSONAL_APP, mockDeletePersonalApp, mockSetAppEnabled, resetMockApps } from "../appsMock";
import { MOCK_APP_HASH, MOCK_APP_SURFACE_ID } from "../personalAppMock";
import { PersonalAppSurface } from "./PersonalAppSurface";

type ToolCall = (tool: string, args: Record<string, unknown>) => Promise<unknown>;
const frame: { onToolCall?: ToolCall } = {};
vi.mock("../mcp-apps/McpAppFrame", () => ({
  McpAppFrame: ({ onToolCall }: { onToolCall: ToolCall }) => { frame.onToolCall = onToolCall; return <div data-testid="view" />; },
  useNearViewport: () => [{ current: null }, true],
}));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement, root: Root;
beforeEach(() => {
  localStorage.clear(); resetMockApps(); resetAppsStore();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  vi.spyOn(bridgeApi, "isolateAppPreviews").mockResolvedValue(true);
  frame.onToolCall = undefined;
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); resetMockApps(); resetAppsStore(); });
const settle = () => act(async () => { await loadApps(); await new Promise(resolve => setTimeout(resolve, 0)); });

it("opens a surface on mount, calls with no chat, and closes it on unmount", async () => {
  const opened = vi.spyOn(bridgeApi, "openPersonalAppSurface");
  const closed = vi.spyOn(bridgeApi, "closePersonalAppSurface");
  const get = vi.spyOn(bridgeApi, "getPersonalAppOpen");
  const call = vi.spyOn(bridgeApi, "callPersonalAppTool");
  act(() => root.render(<PersonalAppSurface appId={MOCK_PERSONAL_APP.id} />));
  await settle(); await settle();
  expect(opened).toHaveBeenCalledWith("slack-dm");
  expect(get).toHaveBeenCalledWith({ sessionId: undefined, openId: MOCK_APP_SURFACE_ID, contentHash: MOCK_APP_HASH });
  expect(container.textContent).toContain("Writes ask first");
  expect(container.textContent, "revising needs a chat").not.toContain("Revise");
  await act(async () => { await frame.onToolCall!("slack_search_users", { query: "Ada" }); });
  expect(call.mock.calls[0][0].sessionId).toBeUndefined();
  act(() => root.render(<div />));
  await settle();
  expect(closed).toHaveBeenCalledWith(MOCK_APP_SURFACE_ID);
});

it("says when the app is off, without opening it, and turns it back on", async () => {
  mockSetAppEnabled(MOCK_PERSONAL_APP.id, false);
  const opened = vi.spyOn(bridgeApi, "openPersonalAppSurface");
  act(() => root.render(<PersonalAppSurface appId={MOCK_PERSONAL_APP.id} />));
  await settle();
  expect(container.textContent).toContain("Slack DM is turned off.");
  expect(opened).not.toHaveBeenCalled();
  const turnOn = [...container.querySelectorAll("button")].find(node => node.textContent === "Turn on")!;
  await act(async () => { turnOn.click(); });
  await settle(); await settle();
  expect(opened).toHaveBeenCalledWith("slack-dm");
});

it("says when the app was deleted", async () => {
  mockDeletePersonalApp(MOCK_PERSONAL_APP.id);
  act(() => root.render(<PersonalAppSurface appId={MOCK_PERSONAL_APP.id} />));
  await settle();
  expect(container.textContent).toContain("This app was deleted.");
});

it("never lets an abandoned open replace a newer pane's open", async () => {
  // The review's case: pane A's open is slow; the person leaves and comes
  // back, so pane B opens. Core replaces the last surface on every open, so
  // B must not start until A has opened and closed itself.
  const order: string[] = [];
  let releaseA!: () => void;
  const opened = vi.spyOn(bridgeApi, "openPersonalAppSurface").mockImplementation(async () => {
    const n = opened.mock.calls.length;
    order.push(`open ${n}`);
    if (n === 1) await new Promise<void>(resolve => { releaseA = resolve; });
    order.push(`opened ${n}`);
    return { openId: `open-${n}`, appId: "slack-dm", version: 1, contentHash: MOCK_APP_HASH };
  });
  const closed = vi.spyOn(bridgeApi, "closePersonalAppSurface").mockImplementation(async openId => { order.push(`close ${openId}`); return { closed: true }; });
  vi.spyOn(bridgeApi, "getPersonalAppOpen").mockRejectedValue(new Error("not needed"));
  act(() => root.render(<PersonalAppSurface key="a" appId={MOCK_PERSONAL_APP.id} />));
  await settle();
  act(() => root.render(<div />));
  act(() => root.render(<PersonalAppSurface key="b" appId={MOCK_PERSONAL_APP.id} />));
  await settle();
  expect(order, "B waits for A").toEqual(["open 1"]);
  releaseA();
  await settle(); await settle();
  expect(order).toEqual(["open 1", "opened 1", "close open-1", "open 2", "opened 2"]);
  expect(closed).not.toHaveBeenCalledWith("open-2");
  expect(container.querySelector("[data-personal-app-surface]")).not.toBeNull();
});
