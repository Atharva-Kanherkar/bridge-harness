// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ConversationItem } from "../conversation";
import { bridgeApi } from "../api";
import { MOCK_APP_DRAFT_ID, MOCK_APP_HASH, mockGetPersonalAppDraft } from "../personalAppMock";
import { PersonalAppDraftCall } from "./PersonalAppDraftCall";
import { VisualActionsContext } from "./VisualCall";
vi.mock("../mcp-apps/McpAppFrame", () => ({ McpAppFrame: ({ personalView }: { personalView: { html: string } }) => <div data-testid="preview">{personalView.html.length} bytes</div>, useNearViewport: () => [{ current: null }, true] }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement, root: Root;
const item: ConversationItem = { key: "k", type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", status: "completed", data: { name: "mcp__bridge__apps_draft", input: { view_html: "untrusted streamed HTML" }, aggregatedOutput: JSON.stringify({ draftId: MOCK_APP_DRAFT_ID, contentHash: MOCK_APP_HASH }) } };
const draft = () => ({ ...mockGetPersonalAppDraft({ sessionId: "session-1", draftId: MOCK_APP_DRAFT_ID, contentHash: MOCK_APP_HASH }), locked: false });
beforeEach(() => { container = document.createElement("div"); document.body.append(container); root = createRoot(container); vi.spyOn(bridgeApi, "getPersonalAppDraft").mockResolvedValue(draft()); });
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
async function render(readOnly = false, ask = vi.fn()) { act(() => root.render(<VisualActionsContext.Provider value={{ ask }}><PersonalAppDraftCall item={item} sessionId="session-1" readOnly={readOnly} /></VisualActionsContext.Provider>)); await settle(); return ask; }
const button = (text: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.includes(text))!;
const click = async (text: string) => { act(() => button(text).click()); await settle(); };
it("fetches canonical content and revises through the composer action", async () => {
  const ask = await render();
  expect(bridgeApi.getPersonalAppDraft).toHaveBeenCalledWith({ sessionId: "session-1", draftId: MOCK_APP_DRAFT_ID, contentHash: MOCK_APP_HASH });
  expect(container.textContent).toContain("Preview only");
  expect(container.textContent).not.toContain("untrusted streamed HTML");
  await click("Revise");expect(ask).toHaveBeenCalledWith(expect.stringContaining("app_id: slack-dm"));
});
it("requires explicit second confirmation before locking wider access", async () => {
  const lock = vi.spyOn(bridgeApi, "lockPersonalApp").mockResolvedValueOnce({ kind: "widens", addedReads: ["slack_get_channel"], addedWrites: [], targetChanged: false });
  await render();await click("Lock version");
  expect(lock).toHaveBeenCalledWith({ draftId: MOCK_APP_DRAFT_ID, contentHash: MOCK_APP_HASH, allowWider: false });
  expect(container.textContent).toContain("New reads: slack_get_channel");expect(lock).toHaveBeenCalledTimes(1);
  lock.mockResolvedValueOnce({ kind: "locked", app: {} as never });
  await click("Allow access and lock");
  expect(lock).toHaveBeenLastCalledWith({ draftId: MOCK_APP_DRAFT_ID, contentHash: MOCK_APP_HASH, allowWider: true });
  expect(container.textContent).toContain("Version locked");
});
it("history disables Lock and Revise, and locked previews remain labelled preview-only", async () => {
  await render(true);expect(button("Lock version").disabled).toBe(true);expect(button("Revise").disabled).toBe(true);
  expect(container.textContent).toContain("Live tools and changes are disabled");
});
it("restores locked state and surfaces fetch/lock errors", async () => {
  vi.mocked(bridgeApi.getPersonalAppDraft).mockResolvedValueOnce({ ...draft(), locked: true });
  await render();expect(button("Version locked").disabled).toBe(true);expect(container.textContent).toContain("Preview only");
});
it("reports unavailable previews without rendering a frame", async () => {
  vi.mocked(bridgeApi.getPersonalAppDraft).mockRejectedValueOnce(new Error("expired"));
  await render();expect(container.textContent).toContain("expired");expect(container.querySelector('[data-testid="preview"]')).toBeNull();
});
it("reports failed locks and allows a retry without changing the hash", async () => {
  vi.spyOn(bridgeApi, "lockPersonalApp").mockRejectedValueOnce(new Error("draft changed"));
  await render();await click("Lock version");expect(container.querySelector('[role="alert"]')?.textContent).toContain("draft changed");expect(button("Lock version").disabled).toBe(false);
});
