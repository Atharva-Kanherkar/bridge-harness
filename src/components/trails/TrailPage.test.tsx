// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MotionGlobalConfig } from "framer-motion";
import type { TrailDetail, TrailEvent, TrailEventKind } from "@/protocol/generated/protocol";
import { TrailPage } from "./TrailPage";
import { TrailStartDialog } from "./TrailStartDialog";
import { TrailAddDialog } from "./TrailAddDialog";
import { AddToTrailDialog } from "./AddToTrailDialog";
import { TrailsScreen } from "../TrailsScreen";
import { mockListTrails, mockReadTrail, resetMockTrails } from "@/trailsMock";
import { mockListNotes, mockReadPage, resetMockNotes } from "@/notesMock";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  MotionGlobalConfig.skipAnimations = true;
  resetMockTrails();
  resetMockNotes();
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
/** Let timers run too: the picker reads its list after a debounce. */
const wait = (ms = 0) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); for (let i = 0; i < 6; i++) await Promise.resolve(); });
const tab = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=tab]")].find(item => item.textContent === label)!;
const candidates = () => [...document.querySelectorAll<HTMLButtonElement>("[role=tabpanel] li button")];
const screen = (trailId: string, onOpenNote = vi.fn()) => act(() => root.render(
  <TrailsScreen trailId={trailId} onSelectTrail={vi.fn()} workspaces={[]} harnesses={[]} onOpenSession={vi.fn()} onOpenNote={onOpenNote} onOpenUrl={vi.fn()} onStart={vi.fn(async () => undefined)} />,
));
const buttons = () => [...document.querySelectorAll("button")];
function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
}
const named = (name: RegExp) => buttons().find(item => name.test(item.getAttribute("aria-label") ?? "") || name.test(item.textContent ?? ""));

function page(trail: TrailDetail, handlers: Partial<Parameters<typeof TrailPage>[0]> = {}) {
  const props = {
    trail, loading: false, onBack: vi.fn(), onContinue: vi.fn(), onOpenEvent: vi.fn(), onRemove: vi.fn(),
    onRename: vi.fn(async () => undefined), onArchive: vi.fn(), onRefresh: vi.fn(), onAdd: vi.fn(), ...handlers,
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

  it("puts the kind label and detail on one meta line under the title", async () => {
    page(mockReadTrail("trail-archive").trail);
    await settle();
    const row = host.querySelector("[data-event-kind=pullRequest]")!;
    const meta = row.querySelector("[data-event-meta]")!;
    expect(meta.textContent).toBe("Pull request·open·added automatically");
    // The meta line is the title's sibling, under it, not a column beside it.
    expect(meta.previousElementSibling?.textContent).toContain("#1012 feat(sidebar): archive a chat from its row");
    expect(row.querySelector(".w-24")).toBeNull();
  });

  it("reserves the remove slot so every time lines up", async () => {
    page(mockReadTrail("trail-archive").trail);
    await settle();
    for (const kind of ["checks", "note", "pullRequest"]) {
      const row = host.querySelector(`[data-event-kind=${kind}]`)!;
      expect(row.querySelector("[data-remove-slot]")).not.toBeNull();
      // Times read in sans tabular figures, never mono.
      expect(row.querySelector("time")?.className).toContain("tabular-nums");
      expect(row.querySelector("time")?.className).not.toContain("font-mono");
    }
    expect(host.querySelector("[data-event-kind=checks] [data-remove-slot] button")).toBeNull();
    expect(host.querySelector("[data-event-kind=note] [data-remove-slot] button")).not.toBeNull();
  });

  it("renders the header from a summary while the detail loads", async () => {
    const summary = mockReadTrail("trail-archive").trail.summary;
    act(() => root.render(<TrailPage trail={null} summary={summary} loading onBack={vi.fn()} onContinue={vi.fn()} onOpenEvent={vi.fn()} onRemove={vi.fn()} onRename={vi.fn(async () => undefined)} onArchive={vi.fn()} onRefresh={vi.fn()} onAdd={vi.fn()} />));
    await settle();
    expect(host.querySelector("h1")?.textContent).toBe("Archive a chat from the sidebar");
    expect(host.textContent).toContain("Needs you");
    expect(host.textContent).toContain("vitest is failing on #1012");
    expect(host.querySelector("[role=status][aria-label='Loading trail']")).not.toBeNull();
    expect(host.querySelector("ol[aria-label=Timeline]")).toBeNull();
    // Nothing known at all: the back link and a header skeleton, not a blank page.
    act(() => root.render(<TrailPage trail={null} loading onBack={vi.fn()} onContinue={vi.fn()} onOpenEvent={vi.fn()} onRemove={vi.fn()} onRename={vi.fn(async () => undefined)} onArchive={vi.fn()} onRefresh={vi.fn()} onAdd={vi.fn()} />));
    await settle();
    expect(named(/^Trails$/)).toBeTruthy();
    expect(host.querySelector("[role=status][aria-label='Loading trail']")).not.toBeNull();
  });

  it("gives each brief item its kind's icon, and the token total in tabular figures", async () => {
    page(mockReadTrail("trail-archive").trail);
    await settle();
    const kinds = [...host.querySelectorAll("[data-brief-kind]")].map(item => item.getAttribute("data-brief-kind"));
    expect(kinds).toEqual(["issue", "note", "pullRequest", "chat", "chat", "message"]);
    const total = [...host.querySelectorAll("section span")].find(item => (item.textContent ?? "").endsWith("tokens"))!;
    expect(total.className).toContain("tabular-nums");
    expect(total.className).not.toContain("font-mono");
  });

  it("says the brief could not be read instead of loading forever", async () => {
    const api = (await import("@/api")).bridgeApi;
    const spy = vi.spyOn(api, "previewTrailBrief").mockRejectedValue(new Error("boom"));
    page(mockReadTrail("trail-archive").trail);
    await settle();
    expect(host.textContent).toContain("Could not read the brief.");
    expect(host.querySelector("[role=status][aria-label='Loading the brief']")).toBeNull();
    spy.mockRestore();
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

  it("opens the Add picker", async () => {
    const props = page(mockReadTrail("trail-archive").trail);
    await settle();
    act(() => named(/^Add$/)!.click());
    expect(props.onAdd).toHaveBeenCalled();
  });

  it("adds an issue, shows the core's refusal, and clears on success", async () => {
    const trail = mockReadTrail("trail-archive").trail.summary;
    const onAddIssue = vi.fn()
      .mockRejectedValueOnce(new Error("That link is a pull request, not an issue. Paste the issue's link, or owner/repo#number."))
      .mockResolvedValueOnce(undefined);
    act(() => root.render(<TrailAddDialog trail={trail} initialKind="issue" onAdd={vi.fn()} onAddIssue={onAddIssue} onCreateNote={vi.fn()} onClose={vi.fn()} />));
    await settle();
    const input = document.querySelector<HTMLInputElement>("input[aria-label='Issue']")!;
    expect(input.placeholder).toBe("Issue link, owner/repo#12, or #12");
    type(input, "https://github.com/acme/bridge/pull/1012");
    await act(async () => named(/^Add$/)!.click());
    await settle();
    expect(onAddIssue).toHaveBeenCalledWith("https://github.com/acme/bridge/pull/1012");
    expect(document.querySelector("form[aria-label='Add issue'] [role=alert]")?.textContent).toContain("pull request, not an issue");
    type(input, " acme/bridge#1020 ");
    await act(async () => named(/^Add$/)!.click());
    await settle();
    expect(onAddIssue).toHaveBeenLastCalledWith("acme/bridge#1020");
    expect(input.value).toBe("");
    expect(document.querySelector("[role=status]")?.textContent).toBe("Added the issue.");
  });

  it("adds an issue through the API from the screen and opens it", async () => {
    const onOpenUrl = vi.fn();
    act(() => root.render(<TrailsScreen trailId="trail-archive" onSelectTrail={vi.fn()} workspaces={[]} harnesses={[]} onOpenSession={vi.fn()} onOpenNote={vi.fn()} onOpenUrl={onOpenUrl} onStart={vi.fn(async () => undefined)} />));
    await settle();
    expect(host.querySelectorAll("[data-event-kind=issue]").length).toBe(1);
    act(() => named(/^Add$/)!.click());
    await wait();
    act(() => tab("Issue").click());
    type(document.querySelector<HTMLInputElement>("input[aria-label='Issue']")!, "#1020");
    await act(async () => document.querySelector<HTMLButtonElement>("form[aria-label='Add issue'] button[type=submit]")!.click());
    await settle();
    expect(host.querySelectorAll("[data-event-kind=issue]").length).toBe(2);
    expect(mockReadTrail("trail-archive").trail.members.some(member => member.kind === "issue" && member.ref === "https://github.com/acme/bridge/issues/1020")).toBe(true);
    act(() => named(/#1004 Archive a chat without losing its history/)!.click());
    expect(onOpenUrl).toHaveBeenCalledWith("https://github.com/acme/bridge/issues/1004");
  });

  it("adds an existing note, a chat and a pull request from the picker", async () => {
    screen("trail-reply");
    await settle();
    act(() => named(/^Add$/)!.click());
    await wait();
    // Notes open first: the person's pages, newest first.
    expect(tab("Note").getAttribute("aria-selected")).toBe("true");
    expect(candidates().map(item => item.getAttribute("aria-label"))).toContain("Add Local-first sync libraries");
    await act(async () => named(/^Add Local-first sync libraries$/)!.click());
    await wait();
    const noteId = mockListNotes().notes.find(note => note.title === "Local-first sync libraries")!.id;
    expect(mockReadTrail("trail-reply").trail.members).toContainEqual(expect.objectContaining({ kind: "note", ref: noteId, addedBy: "person" }));
    expect(document.querySelector("[role=status]")?.textContent).toBe("Added “Local-first sync libraries”.");
    expect(host.querySelector("[data-event-kind=note]")?.textContent).toContain("Local-first sync libraries");
    // It is on this trail now, so the list no longer offers it.
    expect(candidates().map(item => item.getAttribute("aria-label"))).not.toContain("Add Local-first sync libraries");

    act(() => tab("Chat").click());
    await wait();
    // session-2 is already on this trail; session-1 sits on another one.
    const chats = candidates().map(item => item.getAttribute("aria-label"));
    expect(chats).not.toContain("Add Orchestrator");
    expect(chats).toContain("Orchestrator, on the trail “Archive a chat from the sidebar”");
    expect(candidates().find(item => item.getAttribute("aria-label")?.startsWith("Orchestrator"))!.disabled).toBe(true);
    await act(async () => named(/^Add Fix the login redirect loop$/)!.click());
    await wait();
    expect(mockReadTrail("trail-reply").trail.members).toContainEqual(expect.objectContaining({ kind: "session", ref: "session-3" }));

    act(() => tab("Pull request").click());
    await wait();
    type(document.querySelector<HTMLInputElement>("input[aria-label='Search pull requests']")!, "token");
    await wait(200);
    expect(candidates().map(item => item.getAttribute("aria-label"))).toEqual(["Add #1016 fix(auth): keep the token across restarts"]);
    await act(async () => candidates()[0].click());
    await wait();
    expect(mockReadTrail("trail-reply").trail.members).toContainEqual(expect.objectContaining({ kind: "pullRequest", ref: "https://github.com/acme/bridge/pull/1016" }));
    expect(host.querySelector("[data-event-kind=pullRequest]")?.textContent).toContain("#1016 fix(auth): keep the token across restarts");
  });

  it("starts a new note on the trail and opens it", async () => {
    const onOpenNote = vi.fn();
    screen("trail-archive", onOpenNote);
    await settle();
    act(() => named(/^Add$/)!.click());
    await wait();
    type(document.querySelector<HTMLInputElement>("input[aria-label='New note title']")!, "Rollout checklist");
    await act(async () => named(/^New note$/)!.click());
    await wait();
    expect(onOpenNote).toHaveBeenCalledTimes(1);
    const id = onOpenNote.mock.calls[0][0] as string;
    expect(mockReadPage(id).note).toMatchObject({ title: "Rollout checklist", workspaceId: "demo-1" });
    expect(mockReadTrail("trail-archive").trail.members).toContainEqual(expect.objectContaining({ kind: "note", ref: id, addedBy: "person" }));
    expect(document.querySelector("[role=dialog]")).toBeNull();
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

describe("AddToTrailDialog", () => {
  it("puts a note on an open trail", async () => {
    const note = mockListNotes().notes[0];
    const onDone = vi.fn();
    const onClose = vi.fn();
    act(() => root.render(<AddToTrailDialog member={{ kind: "note", ref: note.id }} title="Sync notes" trails={mockListTrails().trails} onDone={onDone} onClose={onClose} />));
    await settle();
    expect(document.body.textContent).toContain("New trail from this note");
    expect(document.body.textContent).not.toContain("Its pull requests, drafts");
    await act(async () => named(/Mission Control stage and queue/)!.click());
    await settle();
    expect(mockReadTrail("trail-stage").trail.members).toContainEqual(expect.objectContaining({ kind: "note", ref: note.id }));
    expect(onDone).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("starts a new trail from a note, in the note's project", async () => {
    const note = mockListNotes().notes[0];
    act(() => root.render(<AddToTrailDialog member={{ kind: "note", ref: note.id }} title="Sync notes" workspaceId="demo-2" trails={[]} onDone={vi.fn()} onClose={vi.fn()} />));
    await settle();
    await act(async () => named(/^Create$/)!.click());
    await settle();
    const created = mockListTrails().trails.find(trail => trail.title === "Sync notes")!;
    expect(created.workspaceId).toBe("demo-2");
    expect(mockReadTrail(created.id).trail.members).toEqual([expect.objectContaining({ kind: "note", ref: note.id })]);
  });

  it("shows why a chat cannot go on a trail", async () => {
    act(() => root.render(<AddToTrailDialog member={{ kind: "session", ref: "session-1" }} title="Orchestrator" trails={mockListTrails().trails} onDone={vi.fn()} onClose={vi.fn()} />));
    await settle();
    expect(document.body.textContent).toContain("Its pull requests, drafts, notes and forks join the trail with it.");
    await act(async () => named(/Outgoing: reply watch/)!.click());
    await settle();
    expect(document.querySelector("[role=alert]")?.textContent).toContain("already on the trail “Archive a chat from the sidebar”");
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
    const chat = document.querySelector<HTMLElement>("[role=checkbox][aria-label='Chat “Implement the archive row action” (Codex)']")!;
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
