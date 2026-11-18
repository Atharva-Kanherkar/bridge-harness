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
