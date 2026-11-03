// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { mockCreatePage, mockListNotes, mockNotesAgentWrite, mockReadPage, resetMockNotes, setMockNotesPassDelay } from "../notesMock";
import type { AdapterDescriptor, Session, Workspace } from "../types";
import type { Editor } from "@tiptap/core";
import { NotesScreen, type NotesScreenProps } from "./NotesScreen";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  resetMockNotes();
  setMockNotesPassDelay(5);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const adapter = (id: string, available = true) => ({ id, label: id, available, authState: "authenticated", capabilities: [], models: [{ id: `${id}-model`, label: id }], defaultModel: `${id}-model` }) as unknown as AdapterDescriptor;
const ADAPTERS = [adapter("codex"), adapter("claude"), adapter("cursor")];
const WORKSPACES = [
  { id: "demo-1", title: "Bridge", status: "ready", dirtyFiles: 0 },
  { id: "demo-2", title: "Landing site", status: "ready", dirtyFiles: 0 },
] as unknown as Workspace[];
const SESSIONS = [{ id: "session-1w", title: "Research sync engines", label: "chat", harness: "claude" }] as unknown as Session[];

function props(overrides: Partial<NotesScreenProps> = {}): NotesScreenProps {
  return {
    adapters: ADAPTERS, workspaces: WORKSPACES, sessions: SESSIONS, defaultHarness: "claude", pollMs: 20,
    onStartChat: vi.fn(async () => "session-new"), onOpenChat: vi.fn(), onError: vi.fn(),
    ...overrides,
  };
}

async function render(overrides: Partial<NotesScreenProps> = {}) {
  const given = props(overrides);
  await act(async () => { root.render(<NotesScreen {...given} />); });
  await settle();
  return given;
}
const settle = async (ms = 0) => { await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); }); };

const buttons = (scope: ParentNode = host) => [...scope.querySelectorAll("button")];
const button = (label: string, scope: ParentNode = host) => buttons(scope).find(node => node.getAttribute("aria-label") === label || node.textContent?.trim() === label);
const click = async (node: Element | undefined) => {
  if (!node) throw new Error("nothing to click");
  await act(async () => { node.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
  await settle();
};
const field = <T extends HTMLElement>(label: string) => host.querySelector<T>(`[aria-label="${label}"]`)!;
const typeInto = async (node: HTMLTextAreaElement | HTMLInputElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(node), "value")!.set!;
  await act(async () => { setter.call(node, value); node.dispatchEvent(new Event("input", { bubbles: true })); });
};
const choose = async (node: HTMLSelectElement, value: string) => {
  await act(async () => { node.value = value; node.dispatchEvent(new Event("change", { bubbles: true })); });
  await settle();
};
const press = async (node: HTMLElement, key: string, init: KeyboardEventInit = {}) => {
  let prevented = false;
  await act(async () => { prevented = !node.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init })); });
  return prevented;
};
const blur = async (node: HTMLElement) => { await act(async () => { node.dispatchEvent(new FocusEvent("focusout", { bubbles: true })); }); await settle(); };
const rowTitles = () => [...host.querySelectorAll("ul[aria-label='Notes'] li button")].map(node => node.textContent ?? "");
const openNote = async (title: string) => click(buttons(host.querySelector("ul[aria-label='Notes']")!).find(node => node.textContent?.startsWith(title)));
/** The page editor. */
const editor = () => (host.querySelector(".note-editor") as HTMLElement & { editor: Editor }).editor;
/** Your words as the editor shows them, in its Markdown. */
const shown = () => editor().getMarkdown().replace(/\s+$/, "");
/** Replace Your words with `markdown`, as if typed. */
const write = async (markdown: string) => { await act(async () => { editor().commands.setContent(markdown, { contentType: "markdown" }); }); };
/** Leave the editor: what was typed goes to the draft, which saves. */
const leave = async () => { await act(async () => { editor().view.dom.dispatchEvent(new FocusEvent("blur")); }); await settle(); };
const optionLabels = (list: Element) => [...list.querySelectorAll("[role='option'] [data-option-label]")].map(node => node.textContent);
const selectedLabel = (list: Element) => list.querySelector("[role='option'][aria-selected='true'] [data-option-label]")?.textContent;
const pick = async (list: Element, label: string) => click([...list.querySelectorAll("[role='option']")].find(node => node.querySelector("[data-option-label]")?.textContent === label));
/** A page property: open its menu and hand back the listbox. */
const property = async (label: string) => {
  await click(button(label, host.querySelector("article dl")!));
  return host.querySelector(`[aria-label='${label} choices'] [role='listbox']`)!;
};

const ONBOARDING = "need to fix the onbaording flow";

describe("capture", () => {
  it("saves exactly what was typed and clears the box before the save returns", async () => {
    const structure = vi.spyOn(bridgeApi, "structureNote");
    const tasks = vi.spyOn(bridgeApi, "extractNoteTasks");
    let release!: () => void;
    const real = bridgeApi.captureNote;
    vi.spyOn(bridgeApi, "captureNote").mockImplementation(body => new Promise(resolve => { release = () => resolve(real(body)); }));
    await render();
    const box = field<HTMLTextAreaElement>("Capture a note");
    const typed = "  teh idea:\n\tkeep my typos  \n";
    await typeInto(box, typed);
    expect(await press(box, "Enter")).toBe(true);
    expect(box.value).toBe("");
    expect(host.querySelector("ul[aria-label='Notes'] li[aria-busy='true']")?.textContent).toContain("Saving…");
    await act(async () => { release(); });
    await settle();
    expect(bridgeApi.captureNote).toHaveBeenCalledWith(typed);
    expect(rowTitles()[0]).toContain("teh idea:");
    await openNote("teh idea:");
    expect(editor().getText()).toContain("teh idea:");
    expect(mockReadPage(mockIdOf("teh idea:")).markdown).toBe(typed);
    expect(structure).not.toHaveBeenCalled();
    expect(tasks).not.toHaveBeenCalled();
  });

  it("keeps Shift+Enter as a newline and ignores an empty capture", async () => {
    const capture = vi.spyOn(bridgeApi, "captureNote");
    await render();
    const box = field<HTMLTextAreaElement>("Capture a note");
    await typeInto(box, "line one");
    expect(await press(box, "Enter", { shiftKey: true })).toBe(false);
    await typeInto(box, "   \n ");
    await press(box, "Enter");
    expect(capture).not.toHaveBeenCalled();
  });

  it("keeps a failed capture in the list with Retry and Discard", async () => {
    vi.spyOn(bridgeApi, "captureNote").mockRejectedValue(new Error("disk full"));
    const given = await render();
    const box = field<HTMLTextAreaElement>("Capture a note");
    await typeInto(box, "keep me");
    await press(box, "Enter");
    await settle();
    const failed = host.querySelector("ul[aria-label='Notes'] li[data-capture='failed']")!;
    expect(failed.textContent).toContain("keep me");
    expect(failed.textContent).toContain("Couldn't save");
    expect(given.onError).toHaveBeenCalledWith("disk full");
    await click(button("Discard", failed));
    expect(host.querySelector("li[data-capture='failed']")).toBeNull();
  });

  it("keeps a failed capture recoverable while the next one is typed", async () => {
    const real = bridgeApi.captureNote;
    let fail!: (error: Error) => void;
    vi.spyOn(bridgeApi, "captureNote")
      .mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }))
      .mockImplementation(body => real(body));
    await render();
    const box = field<HTMLTextAreaElement>("Capture a note");
    await typeInto(box, "thought A");
    await press(box, "Enter");
    await typeInto(box, "thought B, half typed");
    await act(async () => { fail(new Error("offline")); });
    await settle();
    expect(box.value).toBe("thought B, half typed");
    const failed = host.querySelector("ul[aria-label='Notes'] li[data-capture='failed']")!;
    expect(failed.textContent).toContain("thought A");
    await click(button("Retry", failed));
    await settle();
    expect(host.querySelector("li[data-capture='failed']")).toBeNull();
    expect(mockListNotes().notes.some(note => note.preview === "thought A")).toBe(true);
    expect(box.value).toBe("thought B, half typed");
  });

  it("focuses the box when asked from anywhere", async () => {
    await render({ captureRequest: 1 });
    expect(document.activeElement).toBe(field("Capture a note"));
  });
});

describe("list", () => {
  it("shows the results of the newest search when an older one answers last", async () => {
    await render();
    const realList = bridgeApi.listNotes;
    const held: { query: string; resolve: () => void }[] = [];
    vi.spyOn(bridgeApi, "listNotes").mockImplementation(async params => {
      const result = await realList(params);
      await new Promise<void>(resolve => held.push({ query: params?.query ?? "", resolve }));
      return result;
    });
    const search = field<HTMLInputElement>("Search notes");
    await typeInto(search, "onbaording");
    await settle(250);
    await typeInto(search, "sync");
    await settle(250);
    expect(held.map(item => item.query)).toEqual(["onbaording", "sync"]);
    await act(async () => { held[1].resolve(); });
    await settle();
    await act(async () => { held[0].resolve(); });
    await settle();
    expect(rowTitles()).toHaveLength(1);
    expect(rowTitles()[0]).toContain("Local-first sync libraries");
  });
});

describe("pages under pages", () => {
  it("lists a page an agent wrote under another page, in All, in search and on its parent", async () => {
    await render();
    const parentId = mockIdOf(ONBOARDING);
    mockCreatePage({ title: "Onboarding research", content: "what other apps do at step 3", parentId });
    await settle(60);
    expect(rowTitles().some(title => title.includes("Onboarding research") && title.includes("from need to fix"))).toBe(true);
    await typeInto(field<HTMLInputElement>("Search notes"), "Onboarding research");
    await settle(250);
    expect(rowTitles()).toHaveLength(1);
    await typeInto(field<HTMLInputElement>("Search notes"), "");
    await settle(60);
    await openNote(ONBOARDING);
    const pages = host.querySelector("section[aria-label='Pages']")!;
    expect(pages.textContent).toContain("Onboarding research");
    await click(buttons(pages).find(node => node.textContent?.startsWith("Onboarding research")));
    expect(host.querySelector("article")?.getAttribute("aria-label")).toBe("Onboarding research");
  });
});

describe("archive", () => {
  it("finds and restores an archived page", async () => {
    await render();
    await openNote(ONBOARDING);
    await pick(await property("Status"), "Archived");
    await settle(60);
    expect(rowTitles().some(title => title.startsWith(ONBOARDING))).toBe(false);
    await click(buttons().find(node => node.getAttribute("role") === "tab" && node.textContent === "Archived"));
    expect(rowTitles().some(title => title.startsWith(ONBOARDING))).toBe(true);
    await openNote(ONBOARDING);
    await pick(await property("Status"), "Open");
    await settle(60);
    await click(buttons().find(node => node.getAttribute("role") === "tab" && node.textContent === "All"));
    expect(rowTitles().some(title => title.startsWith(ONBOARDING))).toBe(true);
  });
});

describe("history", () => {
  const versionsList = () => document.querySelector("[aria-label='Versions list']")!;
  const entryLabels = () => [...versionsList().querySelectorAll("button")].map(node => node.querySelector("span span.min-w-0")?.textContent);

  it("shows who wrote each version and restores the agent's words", async () => {
    await render();
    await openNote("Local-first sync libraries");
    const original = shown();
    await write(original.replace("Three worth a look", "Two worth a look"));
    await leave();
    await settle(30);
    await click(button("More"));
    await click(button("History"));
    await settle(30);
    expect(entryLabels()).toEqual(["You", "Claude in Research sync engines"]);
    expect(versionsList().querySelector("[aria-current='true']")?.textContent).toContain("Claude in Research sync engines");
    const changes = document.querySelector("[aria-label='Changes']")!;
    expect(changes.querySelector("[data-diff='removed']")?.textContent).toContain("Two worth a look");
    expect(changes.querySelector("[data-diff='added']")?.textContent).toContain("Three worth a look");
    await click(button("Restore this version", document));
    await settle(60);
    expect(document.querySelector("[aria-label='Versions list']")).toBeNull();
    expect(shown()).toBe(original);
    await leave();
    await click(button("More"));
    await click(button("History"));
    await settle(30);
    // The restore is the person's too, minutes after their edit: one run.
    expect(entryLabels()).toEqual(["You", "Claude in Research sync engines"]);
    expect(versionsList().querySelector("button")?.textContent).toContain("2 edits");
  });

  it("stores unsaved typing before a restore", async () => {
    const edit = vi.spyOn(bridgeApi, "editNotePage");
    const restore = vi.spyOn(bridgeApi, "restoreNoteVersion");
    await render();
    await openNote("Local-first sync libraries");
    const original = shown();
    await write(`${original}\n\nunsaved line`);
    await leave();
    await settle(30);
    await write(`${original}\n\nunsaved line\n\nand more`);
    await click(button("More"));
    await click(button("History"));
    await settle(30);
    await click(button("Restore this version", document));
    await settle(60);
    expect(edit).toHaveBeenCalledTimes(2);
    expect(restore).toHaveBeenCalledTimes(1);
    expect(edit.mock.invocationCallOrder[1]).toBeLessThan(restore.mock.invocationCallOrder[0]);
    expect(shown()).toBe(original);
  });
});

describe("Your words", () => {
  it("keeps the blocks you did not touch byte for byte and saves edits with the seq they came from", async () => {
    const edit = vi.spyOn(bridgeApi, "editNotePage");
    await render();
    await openNote(ONBOARDING);
    const page = mockReadPage(mockIdOf(ONBOARDING));
    expect(editor().getText()).toContain("add a skip button");
    // Opening and leaving changes nothing.
    await leave();
    await settle(900);
    expect(edit).not.toHaveBeenCalled();
    await write(`${shown()}\n\nmore`);
    await settle(300);
    expect(field("Save status").textContent).toBe("Unsaved");
    await settle(900);
    // The person's own list style and line breaks are kept; only the new block is new.
    expect(edit).toHaveBeenCalledWith({ id: page.note.id, text: `${page.markdown}\n\nmore`, baseSeq: page.seq, allowDeletingChildren: true });
    expect(field("Save status").textContent).toBe("Edited just now");
    expect(mockReadPage(page.note.id).markdown).toBe(`${page.markdown}\n\nmore`);
  });

  it("reloads on an agent write, and keeps unsaved typing through one", async () => {
    await render();
    await openNote(ONBOARDING);
    const id = mockIdOf(ONBOARDING);
    mockNotesAgentWrite(id, "\nagent line one\n");
    await settle(60);
    expect(shown()).toContain("agent line one");

    const before = shown();
    await write(`MINE ${before}`);
    mockNotesAgentWrite(id, "\nagent line two\n");
    await settle(60);
    expect(shown()).toBe(`MINE ${before}`);
    expect(host.textContent).toContain("This page changed while you were typing");
    await leave();
    await settle(20);
    expect(shown()).toMatch(/^MINE need to fix[\s\S]*agent line one[\s\S]*agent line two/);
    expect(mockReadPage(id).markdown).toMatch(/^MINE need to fix[\s\S]*agent line one[\s\S]*agent line two/);
    expect(host.textContent).not.toContain("This page changed while you were typing");
  });

  it("ignores an older page read that resolves last", async () => {
    await render();
    await openNote(ONBOARDING);
    const id = mockIdOf(ONBOARDING);
    const realRead = bridgeApi.readNotePage;
    const held: { resolve: () => void }[] = [];
    vi.spyOn(bridgeApi, "readNotePage").mockImplementation(async pageId => {
      const page = await realRead(pageId);
      await new Promise<void>(resolve => held.push({ resolve }));
      return page;
    });
    mockNotesAgentWrite(id, "agent line one");
    await settle(60);
    mockNotesAgentWrite(id, "agent line two");
    await settle(60);
    expect(held).toHaveLength(2);
    await act(async () => { held[1].resolve(); });
    await settle();
    await act(async () => { held[0].resolve(); });
    await settle();
    vi.restoreAllMocks();
    expect(shown()).toContain("agent line two");
  });

  it("names the agent and chat that wrote a page, which stays editable", async () => {
    await render();
    await openNote("Local-first sync libraries");
    expect(host.textContent).toContain("Written by Claude in Research sync engines");
    // Drawn as a page, without its leading heading repeating the title.
    const rendered = host.querySelector(".note-editor")!;
    expect(rendered.querySelector("strong")?.textContent).toBe("Yjs");
    expect([...rendered.querySelectorAll("h1")].map(heading => heading.textContent)).not.toContain("Local-first sync libraries");
    expect(host.querySelector("ul[aria-label='Notes'] [aria-current] svg[data-harness='claude']")).not.toBeNull();
    await write("Mine now");
    await leave();
    // The heading stays in the file; the words under it are yours now.
    expect(mockReadPage(mockIdOf("Local-first sync libraries")).markdown).toBe("# Local-first sync libraries\n\nMine now\n");
  });

  it("saves when the person removes a child page block from the page", async () => {
    const id = mockIdOf(ONBOARDING);
    const child = mockCreatePage({ title: "Subpage", content: "x", parentId: id });
    bridgeApi.editNotePage({ id, append: { content: `\n\n<page url="bridge://p/${child.id}">Subpage</page>\n`, position: "end" } });
    const onError = vi.fn();
    await render({ onError });
    await openNote(ONBOARDING);
    expect(host.textContent).toContain("Subpage");
    await write("just words now");
    await leave();
    await settle(30);
    expect(onError).not.toHaveBeenCalled();
    expect(mockReadPage(id).markdown).toBe("just words now\n");
  });
});

describe("notes list", () => {
  const list = () => host.querySelector("aside[aria-label='Notes list']")!;

  it("collapses and expands from its buttons, remembers it, and follows the shortcut", async () => {
    const given = await render();
    expect(list().className).toContain("md:flex");
    await click(button("Hide the notes list"));
    expect(list().className).toContain("md:hidden");
    expect(localStorage.getItem("bridge.notes.listOpen")).toBe("0");
    await click(button("Show the notes list"));
    expect(list().className).toContain("md:flex");

    // ⌘\ reaches the screen as a new request number.
    await act(async () => { root.render(<NotesScreen {...given} listToggleRequest={1} />); });
    expect(list().className).toContain("md:hidden");
    // Collapsed, an open page offers the list back from its header.
    act(() => root.unmount());
    root = createRoot(host);
    await render();
    expect(list().className).toContain("md:hidden");
    await act(async () => { await bridgeApi.listNotes({}); });
    await click(buttons(list()).find(node => node.textContent?.startsWith(ONBOARDING)));
    await click(button("Show the notes list", host.querySelector("nav[aria-label='Breadcrumb']")!));
    expect(list().className).toContain("md:flex");
    expect(localStorage.getItem("bridge.notes.listOpen")).toBe("1");
  });
});

describe("streaming passes", () => {
  it("says it is reading, then fills the Structured tab while Structure runs", async () => {
    setMockNotesPassDelay(300);
    await render();
    await openNote(ONBOARDING);
    await click(button("Structure"));
    await click(button("Structure", host.querySelector("[aria-label='Structure options']")!));
    await settle(30);
    expect(host.textContent).toContain("Reading your words…");
    expect(host.textContent).not.toContain("Structure is opt-in");
    await settle(150);
    const live = host.querySelector("section[aria-label='Structured view'][aria-busy='true']");
    expect(live?.querySelector("h2")?.textContent).toContain("need to fix the onbaording flow");
    expect(live?.textContent).not.toContain("Split into note");
    await settle(300);
    expect(host.querySelector("section[aria-label='Structured view'][aria-busy='true']")).toBeNull();
    expect(host.querySelector("section[aria-label='Structured view']")?.textContent).toContain("Split into note");
  });

  it("shows the tasks Turn into tasks is writing as drafts until it settles", async () => {
    setMockNotesPassDelay(300);
    await render();
    await openNote(ONBOARDING);
    await click(button("Turn into tasks"));
    await click(button("Turn into tasks", host.querySelector("[aria-label='Turn into tasks options']")!));
    await settle(250);
    const drafts = [...host.querySelectorAll("section[aria-label='Tasks'] li[data-draft]")].map(item => item.getAttribute("aria-label"));
    expect(drafts.length).toBeGreaterThan(0);
    expect(drafts[0]).toContain("need to fix the onbaording flow");
    expect(host.querySelector("section[aria-label='Tasks']")?.textContent).toContain("Finding tasks…");
    await settle(300);
    expect(host.querySelectorAll("li[data-draft]")).toHaveLength(0);
    expect(host.querySelector("section[aria-label='Tasks']")?.textContent).toContain("add a skip button");
  });

  it("ignores progress for another page", async () => {
    setMockNotesPassDelay(300);
    await render();
    await openNote("Local-first sync libraries");
    await bridgeApi.structureNote(mockIdOf(ONBOARDING), "claude");
    await settle(250);
    expect(host.querySelector("section[aria-label='Structured view']")).toBeNull();
    expect(host.querySelectorAll("li[data-draft]")).toHaveLength(0);
  });
});

describe("Structure", () => {
  it("adds a second view with quotes, leaves your words alone, and goes stale on an edit", async () => {
    await render();
    await openNote(ONBOARDING);
    const original = shown();
    await click(button("Structure"));
    const picker = host.querySelector("[aria-label='Structure options']")!;
    const harnesses = picker.querySelector("[role='listbox'][aria-label='Harness']")!;
    expect(selectedLabel(harnesses)).toBe("Claude");
    expect(optionLabels(harnesses)).toEqual(["Claude", "Codex"]);
    await click(button("Structure", picker));
    await settle(80);
    const view = host.querySelector("section[aria-label='Structured view']")!;
    expect(view.textContent).toContain("Split into note");
    expect([...view.querySelectorAll("blockquote")].map(quote => quote.textContent)).toContain("add a skip button");
    await click(buttons().find(node => node.getAttribute("role") === "tab" && node.textContent === "Your words"));
    expect(shown()).toBe(original);

    await write(`${original}\n\nnew thought`);
    await leave();
    await click(buttons().find(node => node.getAttribute("role") === "tab" && node.textContent?.startsWith("Structured")));
    expect(host.textContent).toContain("Your words changed since this was built.");
    expect(button("Re-structure")?.getAttribute("aria-label")).toBe("Structure");
  });

  it("runs Structure on the latest words, after the save in flight and the typing since", async () => {
    const realEdit = bridgeApi.editNotePage;
    let release!: () => void;
    const edit = vi.spyOn(bridgeApi, "editNotePage")
      .mockImplementationOnce(params => new Promise(resolve => { release = () => resolve(realEdit(params)); }))
      .mockImplementation(params => realEdit(params));
    let structuredText = "";
    const realStructure = bridgeApi.structureNote;
    const structure = vi.spyOn(bridgeApi, "structureNote").mockImplementation((id, harness) => {
      structuredText = mockReadPage(id).markdown;
      return realStructure(id, harness);
    });
    await render();
    await openNote(ONBOARDING);
    const original = shown();
    await write(`${original}\n\nfirst thought`);
    await leave();
    await write(`${original}\n\nfirst thought\n\nLATEST THOUGHT`);
    await click(button("Structure"));
    await click(button("Structure", host.querySelector("[aria-label='Structure options']")!));
    expect(structure).not.toHaveBeenCalled();
    await act(async () => { release(); });
    await settle(30);
    expect(edit).toHaveBeenCalledTimes(2);
    expect(structure).toHaveBeenCalledTimes(1);
    expect(structuredText).toContain("LATEST THOUGHT");
  });

  it("splits an item into its own note", async () => {
    await render();
    await openNote(ONBOARDING);
    await click(button("Structure"));
    await click(button("Structure", host.querySelector("[aria-label='Structure options']")!));
    await settle(80);
    await click(button('Split "add a skip button" into a note'));
    await settle(60);
    expect(host.textContent).toContain('Saved "add a skip button" as its own note.');
    expect(rowTitles().some(title => title.includes("add a skip button"))).toBe(true);
  });

  it("shows why a pass failed", async () => {
    await render();
    await click(button("New page"));
    await click(button("Structure"));
    await click(button("Structure", host.querySelector("[aria-label='Structure options']")!));
    await settle(120);
    expect(host.querySelector("[role='alert']")?.textContent).toBe("Structure couldn't run: There is nothing to structure yet.");
  });

  it("explains why nothing can run when no harness is signed in, and capture still works", async () => {
    await render({ adapters: [adapter("claude", false), adapter("cursor")] });
    await openNote(ONBOARDING);
    await click(button("Turn into tasks"));
    const picker = host.querySelector("[aria-label='Turn into tasks options']")!;
    expect(picker.textContent).toContain("No harness can run this: sign in to Claude, Codex or OpenCode.");
    expect(button("Turn into tasks", picker)).toBeUndefined();
    const box = field<HTMLTextAreaElement>("Capture a note");
    await typeInto(box, "still works");
    await press(box, "Enter");
    await settle();
    expect(rowTitles()[0]).toContain("still works");
  });
});

describe("tasks and Start", () => {
  async function withTasks(overrides: Partial<NotesScreenProps> = {}) {
    const given = await render(overrides);
    await openNote(ONBOARDING);
    await click(button("Turn into tasks"));
    await click(button("Turn into tasks", host.querySelector("[aria-label='Turn into tasks options']")!));
    await settle(80);
    return given;
  }

  it("lists tasks with their quotes", async () => {
    await withTasks();
    const tasks = host.querySelector("section[aria-label='Tasks']")!;
    expect([...tasks.querySelectorAll("blockquote")].map(quote => quote.textContent)).toEqual([
      "need to fix the onbaording flow, ppl drop at step 3", "add a skip button", "shorter copy on the welcome screen",
    ]);
  });

  it("starts a direct chat with No project, then a chat in a project, and offers Open chat", async () => {
    const given = await withTasks();
    await click(button("Start add a skip button"));
    const picker = host.querySelector("[aria-label='Start a chat']")!;
    const project = picker.querySelector("[role='listbox'][aria-label='Project']")!;
    expect(selectedLabel(project)).toBe("No project");
    expect(optionLabels(project)).toEqual(["No project", "Bridge", "Landing site"]);
    expect(selectedLabel(picker.querySelector("[role='listbox'][aria-label='Chat harness']")!)).toBe("Claude");
    await click(button("Start", picker));
    await settle(60);
    expect(given.onStartChat).toHaveBeenCalledWith(expect.objectContaining({ workspaceId: null, harness: "claude" }));
    const brief = vi.mocked(given.onStartChat).mock.calls[0][0].message;
    expect(brief.startsWith("Task: add a skip button")).toBe(true);
    expect(brief).toContain("> - add a skip button");
    expect(brief).toContain("need to fix the onbaording flow");
    const task = mockReadPage(mockIdOf(ONBOARDING)).tasks.find(item => item.title === "add a skip button")!;
    expect(task).toMatchObject({ state: "doing", sessionIds: ["session-new"] });
    const row = host.querySelector("li[aria-label='add a skip button']")!;
    expect(button("Start add a skip button", row)).toBeUndefined();
    expect(row.querySelector("[role='status']")?.textContent).toBe("Chat linked");
    await click(button("Open chat", row));
    expect(given.onOpenChat).toHaveBeenCalledWith("session-new");

    await click(button("Start shorter copy on the welcome screen"));
    const second = host.querySelector("[aria-label='Start a chat']")!;
    await pick(second.querySelector("[role='listbox'][aria-label='Project']")!, "Landing site");
    await pick(second.querySelector("[role='listbox'][aria-label='Chat harness']")!, "Cursor");
    await click(button("Start", second));
    await settle(60);
    expect(given.onStartChat).toHaveBeenLastCalledWith(expect.objectContaining({ workspaceId: "demo-2", harness: "cursor" }));
    expect(button("Open chat", host.querySelector("li[aria-label='shorter copy on the welcome screen']")!)).toBeDefined();
  });

  it("tracks a linked chat live and lets the person finish and reopen the task", async () => {
    const given = await withTasks({ sessions: [{ ...SESSIONS[0], id: "session-new", status: "working" }] });
    await click(button("Start add a skip button"));
    await click(button("Start", host.querySelector("[aria-label='Start a chat']")!));
    await settle(60);
    const row = () => host.querySelector("li[aria-label='add a skip button']")!;
    expect(row().querySelector("[role='status']")?.textContent).toBe("Running");
    expect(button("Start add a skip button", row())).toBeUndefined();
    for (const [status, label] of [["waiting", "Needs you"], ["ready", "Ready to review"], ["failed", "Failed"], ["stopped", "Stopped"], ["completed", "Finished"]] as const) {
      await act(async () => { root.render(<NotesScreen {...given} sessions={[{ ...SESSIONS[0], id: "session-new", status }]} />); });
      expect(row().querySelector("[role='status']")?.textContent).toBe(label);
      expect(button("Open chat", row())).toBeDefined();
    }
    await click(button("Mark add a skip button done", row()));
    await settle(60);
    expect(row().querySelector("[role='status']")?.textContent).toBe("Done");
    expect(mockReadPage(mockIdOf(ONBOARDING)).tasks.find(task => task.title === "add a skip button")?.state).toBe("done");
    await click(button("Reopen add a skip button", row()));
    await settle(60);
    expect(mockReadPage(mockIdOf(ONBOARDING)).tasks.find(task => task.title === "add a skip button")?.state).toBe("doing");
    expect(row().querySelector("[role='status']")?.textContent).toBe("Finished");
    expect(button("Start add a skip button", row())).toBeUndefined();
    await click(button("add a skip button", row()));
    expect(button("New chat")).toBeDefined();
    expect(button("Start a chat")).toBeUndefined();
  });

  it("creates one chat for repeated Start clicks and allows retry after a failed start", async () => {
    let finish!: (id: string | undefined) => void;
    const onStartChat = vi.fn(() => new Promise<string | undefined>(resolve => { finish = resolve; }));
    await withTasks({ onStartChat });
    await click(button("Start add a skip button"));
    const start = button("Start", host.querySelector("[aria-label='Start a chat']")!)!;
    await act(async () => {
      start.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      start.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await settle();
    expect(onStartChat).toHaveBeenCalledTimes(1);
    expect(host.querySelector("li[aria-label='add a skip button'] [role='status']")?.textContent).toBe("Starting…");
    await act(async () => { finish(undefined); });
    await settle();
    expect(mockReadPage(mockIdOf(ONBOARDING)).tasks.find(task => task.title === "add a skip button")?.sessionIds).toEqual([]);
    await click(button("Start", host.querySelector("[aria-label='Start a chat']")!));
    expect(onStartChat).toHaveBeenCalledTimes(2);
    await click(button("Mark add a skip button done"));
    await act(async () => { finish("session-new"); });
    await settle(60);
    expect(button("Start add a skip button")).toBeUndefined();
    expect(button("Open chat", host.querySelector("li[aria-label='add a skip button']")!)).toBeDefined();
    expect(mockReadPage(mockIdOf(ONBOARDING)).tasks.find(task => task.title === "add a skip button")?.state).toBe("done");
  });

  it("does not offer Start for an unlinked task marked done", async () => {
    await withTasks();
    await click(button("Mark add a skip button done"));
    await settle(60);
    expect(button("Start add a skip button")).toBeUndefined();
    expect(host.querySelector("li[aria-label='add a skip button'] [role='status']")?.textContent).toBe("Done");
    await click(button("Reopen add a skip button"));
    await settle(60);
    expect(button("Start add a skip button")).toBeDefined();
    await click(button("Mark add a skip button done"));
    await settle(60);
    await click(button("add a skip button", host.querySelector("li[aria-label='add a skip button']")!));
    expect(button("Start a chat")).toBeUndefined();
    await pick(await property("Status"), "Archived");
    await settle(60);
    expect(button("Start a chat")).toBeUndefined();
  });

  it("starts a chat with the latest words, even before they autosave", async () => {
    const given = await render();
    await openNote(ONBOARDING);
    const original = shown();
    await write(`${original}\n\nLATEST CHAT REQUIREMENT`);
    await click(button("Start a chat"));
    await click(button("Start", host.querySelector("[aria-label='Start a chat']")!));
    await settle(60);
    expect(given.onStartChat).toHaveBeenCalledTimes(1);
    expect(vi.mocked(given.onStartChat).mock.calls[0][0].message).toContain("LATEST CHAT REQUIREMENT");
  });

  it("does not count archived tasks as completed", async () => {
    await withTasks();
    for (const task of mockReadPage(mockIdOf(ONBOARDING)).tasks) {
      await act(async () => { await bridgeApi.updateNote({ id: task.id, state: "archived" }); });
    }
    await settle(60);
    const tasks = host.querySelector("section[aria-label='Tasks']")!;
    expect(tasks.textContent).toContain("No open tasks");
    expect(tasks.textContent).not.toContain("All done");
    expect(button("Start add a skip button", tasks)).toBeUndefined();
  });

  it("preselects the page's project when it has one", async () => {
    await withTasks();
    await pick(await property("Project"), "Bridge");
    await settle(60);
    await click(button("Start add a skip button"));
    expect(selectedLabel(host.querySelector("[aria-label='Start a chat'] [role='listbox'][aria-label='Project']")!)).toBe("Bridge");
  });

  it("deletes a page with its tasks after saying how many go", async () => {
    await withTasks();
    await click(button("More"));
    await click(button("Delete page"));
    expect(host.querySelector("[role='alertdialog']")?.textContent).toContain("Delete this note and its 3 tasks?");
    await click(button("Delete", host.querySelector("[role='alertdialog']")!));
    await settle(60);
    expect(rowTitles().some(title => title.includes("onbaording"))).toBe(false);
    expect(host.querySelector("textarea[aria-label='Your words']")).toBeNull();
  });
});

describe("projects stay optional", () => {
  it("lists every project plus No project, and links or clears one", async () => {
    const update = vi.spyOn(bridgeApi, "updateNote");
    await render();
    await openNote(ONBOARDING);
    const project = await property("Project");
    expect(optionLabels(project)).toEqual(["No project", "Bridge", "Landing site"]);
    await pick(project, "Landing site");
    expect(update).toHaveBeenLastCalledWith({ id: mockIdOf(ONBOARDING), workspaceId: "demo-2" });
    await settle(60);
    await pick(await property("Project"), "No project");
    expect(update).toHaveBeenLastCalledWith({ id: mockIdOf(ONBOARDING), clearWorkspace: true });
  });

  it("disables nothing when there are no projects", async () => {
    await render({ workspaces: [] });
    await openNote(ONBOARDING);
    expect(optionLabels(await property("Project"))).toEqual(["No project"]);
    for (const label of ["Structure", "Turn into tasks", "Start a chat"]) expect((button(label) as HTMLButtonElement).disabled).toBe(false);
  });
});

function mockIdOf(text: string): string {
  const note = mockListNotes({ includeArchived: true }).notes.find(item => !item.parentId && `${item.title ?? ""}\n${item.preview}`.includes(text));
  if (!note) throw new Error(`no note with ${text}`);
  return note.id;
}
