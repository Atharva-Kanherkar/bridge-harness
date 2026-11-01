import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  mockCaptureNote, mockDeleteNote, mockEditPage, mockExtractTasks, mockListNotes, mockNotesAgentWrite, mockReadPage,
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
