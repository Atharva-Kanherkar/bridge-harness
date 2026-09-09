// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { ArchivedChatsPage } from "./ArchivedChatsPage";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });
const archived = { id: "old", title: "Original work", harness: "codex", workspaceTitle: "Project", archivedAt: "2026-09-09T00:00:00Z" };
it("shows archived chats and unarchives only through the visibility API", async () => {
  vi.spyOn(bridgeApi, "listArchivedChats").mockResolvedValue({ chats: [archived], hasMore: false });
  const restore = vi.spyOn(bridgeApi, "unarchiveChat").mockResolvedValue();
  const start = vi.spyOn(bridgeApi, "startChat");
  await act(async () => { root.render(<ArchivedChatsPage />); });
  expect(host.textContent).toContain("Original work");
  await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="Unarchive Original work"]')!.click(); });
  expect(restore).toHaveBeenCalledWith("old");
  expect(start).not.toHaveBeenCalled();
  expect(host.textContent).toContain("checkout was not restored");
});
it("keeps a failed unarchive visible and reports the error", async () => {
  vi.spyOn(bridgeApi, "listArchivedChats").mockResolvedValue({ chats: [archived], hasMore: false });
  vi.spyOn(bridgeApi, "unarchiveChat").mockRejectedValue(new Error("Database unavailable"));
  await act(async () => { root.render(<ArchivedChatsPage />); });
  await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="Unarchive Original work"]')!.click(); });
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Database unavailable");
  expect(host.textContent).toContain("Original work");
  expect(host.textContent).not.toContain("returned to chat history");
});
it("reads content without starting a model and exposes pagination", async () => {
  vi.spyOn(bridgeApi, "listArchivedChats").mockResolvedValue({ chats: [archived], hasMore: true });
  const replay = vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
  await act(async () => { root.render(<ArchivedChatsPage />); });
  expect(host.textContent).toContain("Next page");
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Original work"))!.click(); });
  expect(replay).toHaveBeenCalledWith("old", 0, 200);
  expect(host.textContent).toContain("Read-only transcript");
});
