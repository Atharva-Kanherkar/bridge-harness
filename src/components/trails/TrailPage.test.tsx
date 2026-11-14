// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MotionGlobalConfig } from "framer-motion";
import type { TrailDetail, TrailEvent, TrailEventKind } from "@/protocol/generated/protocol";
import { TrailPage } from "./TrailPage";
import { TrailStartDialog } from "./TrailStartDialog";
import { TrailsScreen } from "../TrailsScreen";
import { mockReadTrail, resetMockTrails } from "@/trailsMock";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  MotionGlobalConfig.skipAnimations = true;
  resetMockTrails();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = "";
});

const settle = () => act(async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); });
const buttons = () => [...document.querySelectorAll("button")];
function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}
const named = (name: RegExp) => buttons().find(item => name.test(item.getAttribute("aria-label") ?? "") || name.test(item.textContent ?? ""));

function page(trail: TrailDetail, handlers: Partial<Parameters<typeof TrailPage>[0]> = {}) {
  const props = {
    trail, loading: false, onBack: vi.fn(), onContinue: vi.fn(), onOpenEvent: vi.fn(), onRemove: vi.fn(),
    onRename: vi.fn(async () => undefined), onArchive: vi.fn(), onRefresh: vi.fn(), onAddIssue: vi.fn(async () => undefined), ...handlers,
  };
  act(() => root.render(<TrailPage {...props} />));
  return props;
}

describe("TrailPage", () => {
  it("renders every event kind with its label", async () => {
    const trail = mockReadTrail("trail-archive").trail;
    const kinds: TrailEventKind[] = ["issue", "note", "chat", "pullRequest", "checks", "message", "removed"];
    const base = trail.events[0];
    trail.events = kinds.map((kind, index): TrailEvent => ({ ...base, id: `${kind}-${index}`, kind, title: `${kind} title`, harness: undefined }));
    page(trail);
    await settle();
    const rendered = [...host.querySelectorAll("[data-event-kind]")].map(item => item.getAttribute("data-event-kind"));
    expect(rendered).toEqual(kinds);
    for (const label of ["Issue", "Note", "Chat", "Pull request", "Checks", "Message", "Removed"]) expect(host.textContent).toContain(label);
    // A removed record cannot be opened; checks cannot be removed on their own.
    expect(host.querySelector("[data-event-kind=removed] button.text-left")).toBeNull();
    expect(host.querySelector("[data-event-kind=checks] [aria-label^='Remove']")).toBeNull();
  });

  it("removes and opens members, and continues", async () => {
    const trail = mockReadTrail("trail-archive").trail;
    const props = page(trail);
    await settle();
    act(() => named(/Remove this pull request from the trail/)!.click());
    expect(props.onRemove).toHaveBeenCalledWith(expect.objectContaining({ kind: "pullRequest" }));
    act(() => named(/Design the archive action/)!.click());
    expect(props.onOpenEvent).toHaveBeenCalledWith(expect.objectContaining({ title: "Design the archive action" }));
    act(() => named(/Continue this trail/)!.click());
    expect(props.onContinue).toHaveBeenCalled();
    expect(host.textContent).toContain("vitest is failing on #1012");
    expect(host.textContent).toContain("feat/archive-chat");
  });

  it("adds an issue, shows the core's refusal, and closes on success", async () => {
    const trail = mockReadTrail("trail-archive").trail;
    const onAddIssue = vi.fn()
      .mockRejectedValueOnce(new Error("That link is a pull request, not an issue. Paste the issue's link, or owner/repo#number."))
      .mockResolvedValueOnce(undefined);
    page(trail, { onAddIssue });
    await settle();
    act(() => named(/^Add issue$/)!.click());
    const input = host.querySelector<HTMLInputElement>("input[aria-label='Issue']")!;
    expect(input.placeholder).toBe("Issue link, owner/repo#12, or #12");
    type(input, "https://github.com/acme/bridge/pull/1012");
    await act(async () => named(/^Add$/)!.click());
    await settle();
    expect(onAddIssue).toHaveBeenCalledWith("https://github.com/acme/bridge/pull/1012");
    expect(host.querySelector("form[aria-label='Add issue'] [role=alert]")?.textContent).toContain("pull request, not an issue");
    type(input, " acme/bridge#1020 ");
    await act(async () => named(/^Add$/)!.click());
    await settle();
    expect(onAddIssue).toHaveBeenLastCalledWith("acme/bridge#1020");
    expect(host.querySelector("form[aria-label='Add issue']")).toBeNull();
  });

  it("adds an issue through the API from the screen and opens it", async () => {
    const onOpenUrl = vi.fn();
    act(() => root.render(<TrailsScreen trailId="trail-archive" onSelectTrail={vi.fn()} workspaces={[]} harnesses={[]} onOpenSession={vi.fn()} onOpenNote={vi.fn()} onOpenUrl={onOpenUrl} onStart={vi.fn(async () => undefined)} />));
    await settle();
    expect(host.querySelectorAll("[data-event-kind=issue]").length).toBe(1);
    act(() => named(/^Add issue$/)!.click());
    type(host.querySelector<HTMLInputElement>("input[aria-label='Issue']")!, "#1020");
    await act(async () => named(/^Add$/)!.click());
    await settle();
    expect(host.querySelectorAll("[data-event-kind=issue]").length).toBe(2);
    expect(mockReadTrail("trail-archive").trail.members.some(member => member.kind === "issue" && member.ref === "https://github.com/acme/bridge/issues/1020")).toBe(true);
    act(() => named(/#1004 Archive a chat without losing its history/)!.click());
    expect(onOpenUrl).toHaveBeenCalledWith("https://github.com/acme/bridge/issues/1004");
  });

  it("removes through the API from the screen", async () => {
    act(() => root.render(<TrailsScreen trailId="trail-archive" onSelectTrail={vi.fn()} workspaces={[]} harnesses={[]} onOpenSession={vi.fn()} onOpenNote={vi.fn()} onOpenUrl={vi.fn()} onStart={vi.fn(async () => undefined)} />));
    await settle();
    expect(host.querySelectorAll("[data-event-kind=pullRequest]").length).toBe(1);
    act(() => named(/Remove this pull request from the trail/)!.click());
    await settle();
    expect(host.querySelectorAll("[data-event-kind=pullRequest]").length).toBe(0);
    expect(mockReadTrail("trail-archive").trail.members.some(member => member.kind === "pullRequest")).toBe(false);
  });
});

describe("TrailStartDialog", () => {
  it("changes the total and the carried ids when an item is unticked", async () => {
    const onStart = vi.fn(async () => undefined);
    const trail = mockReadTrail("trail-archive").trail.summary;
    act(() => root.render(<TrailStartDialog trail={trail} harnesses={[{ id: "codex", label: "Codex" }, { id: "claude", label: "Claude" }]} defaultHarness="claude" onStart={onStart} onClose={vi.fn()} />));
    await settle();
    const total = () => document.querySelector("[data-testid=brief-total]")?.textContent;
    expect(total()).toBe("6.1k of 14k tokens");
    const chat = document.querySelector<HTMLInputElement>("input[aria-label='Chat “Implement the archive row action” (Codex)']")!;
    act(() => chat.click());
    expect(total()).toBe("3.1k of 14k tokens");
    const area = document.querySelector<HTMLTextAreaElement>("textarea")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    act(() => { setter.call(area, "Fix the check"); area.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => named(/^Start chat$/)!.click());
    expect(onStart).toHaveBeenCalledWith({
      trail,
      text: "Fix the check",
      harness: "claude",
      items: ["issue:https://github.com/acme/bridge/issues/1004", "note:note-plan", "pr:https://github.com/acme/bridge/pull/1012", "chat:session-1w", "message:outgoing-trail-1"],
    });
  });

  it("disables Start over budget", async () => {
    const trail = mockReadTrail("trail-archive").trail.summary;
    const { mockPreviewTrailBrief } = await import("@/trailsMock");
    const preview = mockPreviewTrailBrief("trail-archive");
    preview.items[0].tokens = 20_000;
    const api = await import("@/api");
    const spy = vi.spyOn(api.bridgeApi, "previewTrailBrief").mockResolvedValue({ ...preview, totalTokens: 25_000 });
    act(() => root.render(<TrailStartDialog trail={trail} harnesses={[{ id: "codex", label: "Codex" }]} initialText="go" onStart={vi.fn(async () => undefined)} onClose={vi.fn()} />));
    await settle();
    expect(document.body.textContent).toContain("Untick some items");
    expect((named(/^Start chat$/) as HTMLButtonElement).disabled).toBe(true);
    spy.mockRestore();
  });
});
