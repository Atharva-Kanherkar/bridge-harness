// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { MOCK_APP_HASH, MOCK_APP_OPEN_ID, mockGetPersonalAppOpen } from "../personalAppMock";
import { PersonalAppOpenCard } from "./PersonalAppOpenCall";
import { VisualActionsContext } from "./VisualCall";

type ToolCall = (tool: string, args: Record<string, unknown>) => Promise<unknown>;
const frame: { onToolCall?: ToolCall; input?: unknown } = {};
vi.mock("../mcp-apps/McpAppFrame", () => ({
  McpAppFrame: ({ onToolCall, input }: { onToolCall: ToolCall; input: unknown }) => { frame.onToolCall = onToolCall; frame.input = input; return <div data-testid="view" />; },
  useNearViewport: () => [{ current: null }, true],
}));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement, root: Root;
const open = () => mockGetPersonalAppOpen({ sessionId: "session-1", openId: MOCK_APP_OPEN_ID, contentHash: MOCK_APP_HASH });
const exact = '{"recipient":"Ada","text":"hi"}';
beforeEach(() => {
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  vi.spyOn(bridgeApi, "getPersonalAppOpen").mockResolvedValue(open());
  vi.spyOn(bridgeApi, "isolateAppPreviews").mockResolvedValue(true);
  frame.onToolCall = undefined;
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
async function render(readOnly = false) {
  act(() => root.render(<VisualActionsContext.Provider value={{ ask: vi.fn() }}><PersonalAppOpenCard sessionId="session-1" openId={MOCK_APP_OPEN_ID} contentHash={MOCK_APP_HASH} readOnly={readOnly} /></VisualActionsContext.Provider>));
  await settle();
}
const button = (text: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.includes(text));

it("shows the locked version, its connection and that writes ask first, prefilled", async () => {
  await render();
  expect(container.textContent).toContain("v1");
  expect(container.textContent).toContain("claude.ai Slack · via Claude Code");
  expect(container.textContent).toContain("Writes ask first");
  expect(frame.input).toEqual({ query: "Ada" });
});

it("a write waits in the sheet with the exact JSON and sends only on approval", async () => {
  const call = vi.spyOn(bridgeApi, "callPersonalAppTool")
    .mockResolvedValueOnce({ kind: "approvalRequired", token: "t1", harness: "codex", server: "claude.ai Slack", tool: "slack_send_message", argumentsJson: exact })
    .mockResolvedValueOnce({ kind: "result", result: { content: [{ type: "text", text: "sent" }] } });
  await render();
  let result: unknown;
  const waiting = frame.onToolCall!("slack_send_message", { text: "hi", recipient: "Ada" }).then(value => { result = value; });
  await settle();
  expect(container.querySelector('[aria-label="Exact arguments"]')?.textContent).toBe(exact);
  expect(container.textContent).toContain("slack_send_message on claude.ai Slack via Codex");
  expect(call).toHaveBeenCalledTimes(1);
  expect(call.mock.calls[0][0].approvalToken).toBeUndefined();
  await expect(frame.onToolCall!("slack_send_message", { text: "again" })).rejects.toThrow("waiting for approval");
  expect(call).toHaveBeenCalledTimes(1);
  act(() => button("Send")!.click());
  await settle(); await waiting;
  expect(call).toHaveBeenLastCalledWith(expect.objectContaining({ tool: "slack_send_message", approvalToken: "t1" }));
  expect(result).toEqual({ content: [{ type: "text", text: "sent" }] });
  expect(container.querySelector('[aria-label="Exact arguments"]')).toBeNull();
});

it("cancelling refuses the view's call and sends nothing", async () => {
  const call = vi.spyOn(bridgeApi, "callPersonalAppTool").mockResolvedValueOnce({ kind: "approvalRequired", token: "t1", harness: "claude", server: "claude.ai Slack", tool: "slack_send_message", argumentsJson: exact });
  await render();
  const waiting = frame.onToolCall!("slack_send_message", { text: "hi" });
  await settle();
  act(() => button("Cancel")!.click());
  await expect(waiting).rejects.toThrow("declined");
  expect(call).toHaveBeenCalledTimes(1);
});

it("reads answer at once, need isolation, and history is read-only", async () => {
  const call = vi.spyOn(bridgeApi, "callPersonalAppTool").mockResolvedValue({ kind: "result", result: { content: [{ type: "text", text: "Ada" }] } });
  await render();
  await expect(frame.onToolCall!("slack_search_users", { query: "Ada" })).resolves.toEqual({ content: [{ type: "text", text: "Ada" }] });
  act(() => root.unmount()); root = createRoot(container);
  vi.mocked(bridgeApi.isolateAppPreviews).mockResolvedValueOnce(false);
  await render();
  await expect(frame.onToolCall!("slack_search_users", {})).rejects.toThrow("Bridge Mac app");
  act(() => root.unmount()); root = createRoot(container);
  await render(true);
  await expect(frame.onToolCall!("slack_send_message", {})).rejects.toThrow("history");
  expect(button("Revise")!.disabled).toBe(true);
  expect(call).toHaveBeenCalledTimes(1);
});
