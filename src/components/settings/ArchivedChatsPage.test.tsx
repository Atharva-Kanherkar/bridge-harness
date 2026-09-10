// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { ArchivedChatsPage } from "./ArchivedChatsPage";
import { MotionGlobalConfig } from "framer-motion";
import { asWireKind } from "../../transcript/wire";
import type { AgentEvent } from "../../types";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  MotionGlobalConfig.skipAnimations = true;
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); MotionGlobalConfig.skipAnimations = false; });
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

const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "old", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
  role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {},
  createdAt: "2026-09-09T00:00:00Z", ...overrides,
});
const openRoot = async () => {
  await act(async () => { root.render(<ArchivedChatsPage />); });
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Original work"))!.click(); });
};
const listRoots = () => vi.spyOn(bridgeApi, "listArchivedChats").mockImplementation(async (_query, _offset, root) => ({ chats: root ? [] : [archived], hasMore: false }));

it("renders a non-message history page through the shared transcript with disabled approvals", async () => {
  listRoots();
  vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([
    event(1, "reasoning.completed", { text: "Historical reasoning survives" }),
    event(2, "command.completed", { title: "git status", data: { command: "git status", exitCode: 0 }, text: "Stored tool output" }),
    event(3, "file_change.completed", { title: "archive.ts", data: { path: "src/archive.ts", patch: "@@ -1 +1 @@\n-before\n+archiveFixed", additions: 1, deletions: 1 } }),
    event(4, "error", { text: "Saved execution failed", status: "failed" }),
    event(5, "approval.requested", { title: "Historical approval", status: "pending", data: { command: "bun test" } }),
    event(6, "delegation.spawned", { title: "Archived worker", text: "Saved delegation objective", data: { childSessionId: "child" } }),
    event(7, "permission.requested", { title: "Historical permission", status: "pending", data: { actions: [{ decision: "accept", label: "Allow tool" }] } }),
    event(8, "question.requested", { title: "Historical question", status: "pending", data: { questions: [{ id: "q", question: "Saved choice", options: ["A", "B"] }] } }),
    event(9, "approval.requested", { title: "Prompt guidance", status: "pending", data: { approvalType: "prompt_mutation", target: "worker:implementation", beforeText: "Before", afterText: "After" } }),
  ]);
  const resolve = vi.spyOn(bridgeApi, "resolveApproval");
  await openRoot();
  expect(host.textContent).toContain("Historical approval");
  expect(host.textContent).toContain("Saved execution failed");
  expect(host.textContent).toContain("archiveFixed");
  expect(host.textContent).toContain("Archived worker");
  const thinking = host.querySelector<HTMLElement>('[data-thinking="completed"] summary');
  expect(thinking).not.toBeNull();
  await act(async () => { thinking!.click(); });
  expect(host.textContent).toContain("Historical reasoning survives");
  const allow = [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Allow once"))!;
  expect(allow.matches(":disabled")).toBe(true);
  const interactions = host.querySelectorAll('fieldset[aria-label="Historical interaction, read-only"]');
  expect(interactions).toHaveLength(4);
  expect([...interactions].flatMap(item => [...item.querySelectorAll("button,input")]).every(control => control.matches(":disabled"))).toBe(true);
  await act(async () => { allow.click(); });
  expect(resolve).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain("Design preview");
});

it("reads descendants under their root and only unarchives that root explicitly", async () => {
  const child = { ...archived, id: "child", title: "Worker trace" };
  const list = vi.spyOn(bridgeApi, "listArchivedChats").mockImplementation(async (_query, _offset, root) => ({ chats: root ? [child] : [archived], hasMore: false }));
  const replay = vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([event(1, "message.completed", { role: "assistant", text: "Saved content" })]);
  const restore = vi.spyOn(bridgeApi, "unarchiveChat").mockResolvedValue();
  const start = vi.spyOn(bridgeApi, "startChat");
  await openRoot();
  expect(list).toHaveBeenCalledWith("", 0, "old");
  await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="Read Worker trace"]')!.click(); });
  expect(replay).toHaveBeenLastCalledWith("child", 0, 200);
  expect(restore).not.toHaveBeenCalled();
  expect(start).not.toHaveBeenCalled();
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent?.trim() === "Unarchive root chat")!.click(); });
  expect(restore).toHaveBeenCalledWith("old");
});

it("keeps preceding events when loading a tool completion from the next history page", async () => {
  listRoots();
  const page = Array.from({ length: 200 }, (_, i) => event(i + 1, "session.status", { status: "idle" }));
  page[0] = event(1, "message.completed", { role: "user", text: "First page request" });
  page[199] = event(200, "command.started", { itemId: "spanning", title: "echo span", status: "inProgress", data: { command: "echo span" } });
  const replay = vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValueOnce(page).mockResolvedValueOnce([
    event(201, "command.completed", { itemId: "spanning", title: "echo span", data: { command: "echo span", exitCode: 0 }, text: "Completed across pages" }),
  ]);
  await openRoot();
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Load more history"))!.click(); });
  expect(replay).toHaveBeenLastCalledWith("old", 200, 200);
  expect(host.textContent).toContain("First page request");
  expect(host.querySelectorAll("[data-activity-group]")).toHaveLength(1);
  const activity = host.querySelector<HTMLButtonElement>("[data-activity-group] > button")!;
  expect(activity.textContent).toContain("1 step");
  expect(activity.textContent).not.toContain("Working");
  await act(async () => { activity.click(); });
  expect(activity.getAttribute("aria-expanded")).toBe("true");
  expect(host.textContent).not.toContain("Load more history");
});

it("explains a metadata-only history page instead of rendering a blank body", async () => {
  listRoots();
  vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([event(1, "session.status", { status: "idle" })]);
  await openRoot();
  expect(host.textContent).toContain("No displayable transcript content");
});
