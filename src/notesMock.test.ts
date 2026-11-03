import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  mockCaptureNote, mockCreatePage, mockUpdateNote, mockDeleteNote, mockEditPage, mockExtractTasks, mockListNotes, mockNotesAgentWrite, mockReadPage,
  mockPageHistory, mockReadVersion, mockRestoreVersion, mockStructureNote, onMockNotesPageChanged, resetMockNotes, setMockNotesPassDelay,
} from "./notesMock";

beforeEach(() => { resetMockNotes(); vi.useFakeTimers(); setMockNotesPassDelay(10); });
afterEach(() => { vi.useRealTimers(); });

describe("mock notes", () => {
  it("captures exactly what was typed into the Inbox", () => {
    const body = "  teh idea:\n\tkeep typos  \n";
    const note = mockCaptureNote(body);
    expect(note.state).toBe("inbox");
    expect(mockReadPage(note.id).markdown).toBe(body);
    expect(mockListNotes().notes[0].id).toBe(note.id);
  });

  it("moves and promotes pages while refusing cycles", () => {
    const parent = mockCreatePage({ title: "Parent" });
    const child = mockCaptureNote("Child");
    expect(mockReadPage(parent.id).backlinks).toEqual([]);
    expect(mockUpdateNote({ id: child.id, parentId: parent.id })).toMatchObject({ parentId: parent.id, state: "open" });
    expect(() => mockUpdateNote({ id: parent.id, parentId: child.id })).toThrow("ancestor");
    expect(mockUpdateNote({ id: child.id, clearParent: true }).parentId).toBeNull();
    expect(() => mockUpdateNote({ id: child.id, clearParent: true, parentId: parent.id })).toThrow("mutually exclusive");
  });

  it("finds backlinks from wiki links and page tags, quoting the link's line", () => {
    const plan = mockCreatePage({ title: "Launch plan", content: "the plan" });
    const wiki = mockCreatePage({ content: "See [[launch plan]] and go." });
    const tagged = mockCreatePage({ title: "Rollout", content: `<page url="bridge://p/${plan.id}">Launch plan</page>` });
    expect(mockReadPage(plan.id).backlinks).toEqual(expect.arrayContaining([
      expect.objectContaining({ pageId: wiki.id, kind: "wiki", excerpt: "See launch plan and go." }),
      expect.objectContaining({ pageId: tagged.id, kind: "page", excerpt: "Launch plan" }),
    ]));

    // Case aside, the link resolves, and code never becomes one.
    const cased = mockCaptureNote("Plans: [[Launch plan|the launch]]!\ncode:\n\n```\n[[Launch plan]]\n```\n");
    expect(mockReadPage(plan.id).backlinks.find(row => row.pageId === cased.id)).toMatchObject({ kind: "wiki", excerpt: "Plans: the launch!" });
    expect(mockReadPage(plan.id).backlinks).toHaveLength(3);

    // A title two pages can hold sends its link to the first one, and that
    // page's rename alone carries the link.
    const other = mockCreatePage({ title: "Launch plan", content: "duplicate" });
    mockUpdateNote({ id: other.id, title: "Renamed" });
    const rows = mockReadPage(plan.id).backlinks;
    expect(rows.find(row => row.pageId === wiki.id)).toBeTruthy();
  });

  it("carries wiki links and page tags with a rename, and leaves code alone", () => {
    const target = mockCreatePage({ title: "Old", content: "First." });
    const source = mockCreatePage({ content: `[[old]] and [[old|mine]] plus <page url="bridge://p/${target.id}">Old</page>` });
    mockUpdateNote({ id: target.id, title: "New Title!" });
    expect(mockReadPage(source.id).markdown).toBe(`[[New Title!]] and [[New Title!|mine]] plus <page url="bridge://p/${target.id}">New Title!</page>`);

    // The rename does not touch code blocks, and the next rename still moves
    // the links that now name the new title.
    const fenced = mockCaptureNote("code now\n```\n[[New Title!]]\n```\nstill itself");
    mockUpdateNote({ id: target.id, title: "Part Two" });
    expect(mockReadPage(source.id).markdown).toContain("[[Part Two]]");
    expect(mockReadPage(fenced.id).markdown.includes("```\n[[New Title!]]\n```")).toBe(true);

    // A title the wiki grammar cannot hold becomes a page tag, alias kept.
    mockUpdateNote({ id: target.id, title: "Q4 | Launches" });
    expect(mockReadPage(source.id).markdown).toBe(`<page url="bridge://p/${target.id}">Q4 | Launches</page> and <page url="bridge://p/${target.id}">mine</page> plus <page url="bridge://p/${target.id}">Q4 | Launches</page>`);
  });

  it("carries page tags with a rename even where no wiki link names the page", () => {
    const first = mockCreatePage({ title: "Shared", content: "oldest" });
    const target = mockCreatePage({ title: "Shared", content: "newer" });
    const tagOnly = mockCreatePage({ content: `see <page url="bridge://p/${target.id}">Shared</page>` });
    const wiki = mockCreatePage({ content: "[[Shared]]" });
    mockUpdateNote({ id: target.id, title: "Moved" });
    expect(mockReadPage(tagOnly.id).markdown).toBe(`see <page url="bridge://p/${target.id}">Moved</page>`);
    // `[[Shared]]` resolves to the oldest page, so this rename leaves it be.
    expect(mockReadPage(wiki.id).markdown).toBe("[[Shared]]");
    expect(mockReadPage(first.id).note.title).toBe("Shared");
  });

  it("rewrites a page tag and a wiki link on one line without shifting either", () => {
    const target = mockCreatePage({ title: "Old", content: "x" });
    const source = mockCreatePage({ content: `<page url="bridge://p/${target.id}">Old</page> see [[Old]] end` });
    mockUpdateNote({ id: target.id, title: "A much longer title" });
    expect(mockReadPage(source.id).markdown).toBe(`<page url="bridge://p/${target.id}">A much longer title</page> see [[A much longer title]] end`);
  });

  it("reads a wiki link inside a page tag's text as the tag alone", () => {
    const target = mockCreatePage({ title: "Plan", content: "x" });
    const source = mockCreatePage({ content: `<page url="bridge://p/${target.id}">[[Plan]]</page> after` });
    mockUpdateNote({ id: target.id, title: "Launch plan" });
    expect(mockReadPage(source.id).markdown).toBe(`<page url="bridge://p/${target.id}">Launch plan</page> after`);
  });

  it("sends a shared title's backlink to the oldest page only, untitled pages by first line", () => {
    const oldest = mockCreatePage({ title: "Spec", content: "one" });
    const newer = mockCreatePage({ title: "Spec", content: "two" });
    const untitled = mockCreatePage({ content: "Roadmap\nlater" });
    mockCreatePage({ content: "see [[Spec]] and [[roadmap]]" });
    expect(mockReadPage(oldest.id).backlinks).toHaveLength(1);
    expect(mockReadPage(newer.id).backlinks).toHaveLength(0);
    expect(mockReadPage(untitled.id).backlinks).toHaveLength(1);
  });

  it("merges a stale save with an agent write", () => {
    const note = mockCaptureNote("first line\n");
    const base = mockReadPage(note.id).seq;
    mockNotesAgentWrite(note.id, "agent line\n");
    mockEditPage({ id: note.id, baseSeq: base, text: "my first line\n" });
    expect(mockReadPage(note.id).markdown).toBe("my first line\nagent line\n");
  });

  it("structures by quoting the body and goes stale after an edit", () => {
    const note = mockCaptureNote("one thought\nanother thought\n");
    mockStructureNote({ id: note.id, harness: "codex" });
    vi.advanceTimersByTime(20);
    const page = mockReadPage(note.id);
    expect(page.structured?.items.map(item => item.excerpt)).toEqual(["one thought", "another thought"]);
    expect(page.structured?.stale).toBe(false);
    expect(page.markdown).toBe("one thought\nanother thought\n");
    expect(mockListNotes().jobs[0]).toMatchObject({ kind: "structure", status: "done", harness: "codex" });
    mockEditPage({ id: note.id, baseSeq: page.seq, text: "one thought, edited\nanother thought\n" });
    expect(mockReadPage(note.id).structured?.stale).toBe(true);
  });

  it("turns lines into tasks that quote the page, once", () => {
    const note = mockCaptureNote("plan\n- write the doc\nwe need to ship it\n");
    mockExtractTasks({ id: note.id });
    vi.advanceTimersByTime(20);
    mockExtractTasks({ id: note.id });
    vi.advanceTimersByTime(20);
    const tasks = mockReadPage(note.id).tasks;
    expect(tasks.map(task => task.sourceExcerpt)).toEqual(["- write the doc", "we need to ship it"]);
    expect(tasks.every(task => task.kind === "task" && task.state === "open" && task.parentId === note.id)).toBe(true);
  });

  it("deletes a page with its tasks", () => {
    const note = mockCaptureNote("- a task\n");
    mockExtractTasks({ id: note.id });
    vi.advanceTimersByTime(20);
    mockDeleteNote(note.id);
    expect(mockListNotes().notes.some(item => item.id === note.id || item.parentId === note.id)).toBe(false);
  });

  it("keeps edited and started tasks when more work is extracted", () => {
    const note = mockCaptureNote("- write the doc\n- write the doc\n");
    mockExtractTasks({ id: note.id });
    vi.advanceTimersByTime(20);
    const [first] = mockReadPage(note.id).tasks;
    expect(mockReadPage(note.id).tasks).toHaveLength(1);
    mockUpdateNote({ id: first.id, title: "My custom title", state: "done", addSessionId: "chat-existing" });
    const existing = mockReadPage(first.id).note;
    mockEditPage({ id: note.id, text: "- write   the doc\n- ship the app\n" });
    mockExtractTasks({ id: note.id });
    vi.advanceTimersByTime(20);
    const tasks = mockReadPage(note.id).tasks;
    expect(tasks).toHaveLength(2);
    expect(tasks.find(task => task.id === first.id)).toEqual(existing);
    expect(tasks.find(task => task.id !== first.id)?.sourceExcerpt).toBe("- ship the app");
  });

  it("announces every write", () => {
    const changes: string[] = [];
    onMockNotesPageChanged(change => changes.push(change.actor));
    const note = mockCaptureNote("x");
    mockNotesAgentWrite(note.id, "y");
    expect(changes).toEqual(["person", "agent:claude:session-1w"]);
  });
});

describe("mock history", () => {
  it("records who wrote each version and restores one", () => {
    const note = mockCaptureNote("first words");
    mockNotesAgentWrite(note.id, "agent line");
    const versions = mockPageHistory(note.id).versions;
    expect(versions.map(version => version.actor)).toEqual(["agent:claude:session-1w", "person"]);
    expect(versions[0]).toMatchObject({ bodyChanged: true, added: 11, removed: 0 });
    expect(mockReadVersion(note.id, 1).markdown).toBe("first words");
    expect(mockRestoreVersion(note.id, 1).warnings).toEqual([]);
    expect(mockReadPage(note.id).markdown).toBe("first words");
    expect(mockPageHistory(note.id).versions[0].actor).toBe("person");
    expect(mockRestoreVersion(note.id, 1).warnings).toEqual(["nothing changed"]);
  });
});
