import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import type { NoteProperty } from "./protocol/generated/protocol";
import {
  mockCreateDatabase, mockDeleteView, mockQueryDatabase, mockSaveView, mockUpdateSchema,
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

interface SharedCases {
  today: string;
  schema: { name: string; type: string; options?: (string | [string, string])[]; dateFormat?: string }[];
  rows: { title: string; kind: string; state: string; tags: string[]; values: Record<string, unknown> }[];
  cases: { name: string; filter?: unknown; sorts?: { property: string; direction: string }[]; groupBy?: { property: string; dateBucket?: string }; expect: string[]; groups?: [string, number][] }[];
}

describe("mock databases", () => {
  it("mock queries match the shared fixture", () => {
    const shared = JSON.parse(readFileSync(new URL("../testing/fixtures/notes-query-cases.json", import.meta.url), "utf8")) as SharedCases;
    const page = mockCreateDatabase({
      title: "Fixture",
      properties: shared.schema.map(property => ({
        name: property.name,
        type: property.type,
        dateFormat: property.dateFormat ?? null,
        options: property.options?.map(option => (Array.isArray(option) ? { name: option[0], group: option[1] } : { name: option })) ?? null,
      })),
    });
    const id = page.note.id;
    const properties = page.database!.properties;
    const byName = (name: string): NoteProperty | undefined => properties.find(property => property.name === name);
    const optionId = (property: NoteProperty, name: unknown) => property.options?.find(option => option.name === name)?.id ?? name;
    for (const row of shared.rows) {
      const created = mockCreatePage({ title: row.title, parentId: id, kind: row.kind, tags: row.tags });
      const values: Record<string, unknown> = {};
      for (const [name, value] of Object.entries(row.values)) {
        const property = byName(name)!;
        values[property.id] = property.type === "date" ? { start: value }
          : property.type === "multiSelect" ? (value as string[]).map(item => optionId(property, item))
          : ["select", "status"].includes(property.type) ? optionId(property, value)
          : value;
      }
      mockUpdateNote({ id: created.id, state: row.state, ...(Object.keys(values).length ? { values } : {}) });
    }
    const named = (tree: unknown): unknown => {
      const node = tree as Record<string, unknown>;
      for (const key of ["and", "or"]) if (key in node) return { [key]: (node[key] as unknown[]).map(named) };
      const property = byName(node.property as string);
      const condition: Record<string, unknown> = { property: property?.id ?? node.property, op: node.op };
      if ("value" in node) {
        const option = (raw: unknown) => (property && ["select", "status", "multiSelect"].includes(property.type) && node.op !== "inGroup" ? optionId(property, raw) : raw);
        condition.value = Array.isArray(node.value) ? node.value.map(option) : option(node.value);
      }
      return condition;
    };
    for (const sharedCase of shared.cases) {
      const property = (name: string) => byName(name)?.id ?? name;
      const result = mockQueryDatabase({
        databaseId: id,
        filter: sharedCase.filter ? named(sharedCase.filter) : null,
        sorts: sharedCase.sorts?.map(sort => ({ property: property(sort.property), direction: sort.direction })) ?? null,
        groupBy: sharedCase.groupBy ? { property: property(sharedCase.groupBy.property), dateBucket: sharedCase.groupBy.dateBucket ?? null } : null,
        today: shared.today,
        limit: 500,
      });
      expect(result.rows.map(row => row.note.title), sharedCase.name).toEqual(sharedCase.expect);
      if (sharedCase.groups) expect(result.groups!.map(group => [group.label, group.count]), sharedCase.name).toEqual(sharedCase.groups);
    }
  });

  it("creates a database with a status and a table, and hides the system database", () => {
    const page = mockCreateDatabase({ title: "Reading list" });
    expect(page.note.kind).toBe("database");
    expect(page.database!.properties.map(property => [property.name, property.type])).toEqual([["Status", "status"]]);
    expect(page.database!.properties[0].options!.map(option => [option.name, option.group])).toEqual([["To do", "todo"], ["In progress", "inProgress"], ["Done", "done"]]);
    expect(page.database!.views.map(view => view.name)).toEqual(["Table"]);
    const list = mockListNotes({ includeArchived: true });
    expect(list.systemDatabaseId).toBeTruthy();
    expect(list.notes.some(note => note.id === list.systemDatabaseId)).toBe(false);
    expect(mockReadPage(list.systemDatabaseId!).database!.views.map(view => view.name)).toEqual(["All", "Inbox", "Pages", "Tasks", "Ideas", "In progress with an agent", "Archived"]);
  });

  it("changes a schema, keeps option ids, and clears the values of dropped options", () => {
    const page = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }, { name: "Genre", type: "select", options: [{ name: "Fiction" }, { name: "Tech" }] }] });
    const [rating, genre] = page.database!.properties;
    const [fiction, tech] = genre.options!;
    const row = mockCreatePage({ title: "DDIA", parentId: page.note.id });
    mockUpdateNote({ id: row.id, values: { [rating.id]: 5, [genre.id]: tech.id } });
    const read = mockReadPage(row.id);
    expect(read.databaseId).toBe(page.note.id);
    expect(read.values).toEqual({ [rating.id]: 5, [genre.id]: tech.id });
    expect(() => mockUpdateNote({ id: row.id, values: { [genre.id]: "nope" } })).toThrow("Fiction, Tech");
    const schema = mockUpdateSchema({ id: page.note.id, changes: [
      { op: "rename", propertyId: rating.id, name: "Score" },
      { op: "setOptions", propertyId: genre.id, options: [{ id: fiction.id, name: "Novels" }, { name: "History" }] },
      { op: "reorder", propertyIds: [genre.id, rating.id] },
    ] });
    expect(schema.properties.map(property => property.name)).toEqual(["Genre", "Score"]);
    expect(schema.properties[0].options!.map(option => [option.id === fiction.id, option.name])).toEqual([[true, "Novels"], [false, "History"]]);
    expect(mockReadPage(row.id).values).toEqual({ [rating.id]: 5 });
    mockUpdateNote({ id: row.id, values: { [rating.id]: null } });
    expect(mockReadPage(row.id).values).toEqual({});
    expect(() => mockUpdateSchema({ id: page.note.id, changes: [{ op: "reorder", propertyIds: [rating.id] }] })).toThrow("every property once");
  });

  it("saves and deletes views, keeping one, and refuses a view that would not run", () => {
    const page = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }] });
    const [rating] = page.database!.properties;
    const view = mockSaveView({ databaseId: page.note.id, view: { name: "Top", type: "board", filter: { property: rating.id, op: "gte", value: 4 } } });
    expect(mockReadPage(page.note.id).database!.views.map(candidate => candidate.name)).toEqual(["Table", "Top"]);
    expect(() => mockSaveView({ databaseId: page.note.id, view: { name: "Bad", type: "list", filter: { property: rating.id, op: "contains", value: "x" } } })).toThrow("eq, ne");
    mockDeleteView({ databaseId: page.note.id, viewId: page.database!.views[0].id });
    expect(() => mockDeleteView({ databaseId: page.note.id, viewId: view.id })).toThrow("at least one view");
  });

  it("pages with a cursor and lets the system views reproduce the Notes list", () => {
    const page = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }] });
    const rating = page.database!.properties[0].id;
    for (let index = 0; index < 7; index++) {
      const row = mockCreatePage({ title: `Book ${index}`, parentId: page.note.id });
      mockUpdateNote({ id: row.id, values: { [rating]: index % 3 } });
    }
    const sorts = [{ property: rating, direction: "asc" }, { property: "title", direction: "asc" }];
    const first = mockQueryDatabase({ databaseId: page.note.id, sorts, limit: 3 });
    expect(first.total).toBe(7);
    const second = mockQueryDatabase({ databaseId: page.note.id, sorts, limit: 3, cursor: first.nextCursor });
    const third = mockQueryDatabase({ databaseId: page.note.id, sorts, limit: 3, cursor: second.nextCursor });
    expect([...first.rows, ...second.rows, ...third.rows].map(row => row.note.title)).toEqual(["Book 0", "Book 3", "Book 6", "Book 1", "Book 4", "Book 2", "Book 5"]);
    expect(third.nextCursor).toBeUndefined();

    const { systemDatabaseId } = mockListNotes();
    const inbox = mockReadPage(systemDatabaseId!).database!.views.find(view => view.name === "Inbox")!;
    const listed = mockListNotes().notes.filter(note => note.state === "inbox" && !(note.parentId && note.kind === "task")).map(note => note.id).sort();
    expect(mockQueryDatabase({ databaseId: systemDatabaseId!, viewId: inbox.id, limit: 500 }).rows.map(row => row.note.id).sort()).toEqual(listed);
  });

  it("pages by a title in any script, refuses what the core refuses, and returns only live cells", () => {
    const page = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }, { name: "Genre", type: "select", options: [{ name: "Fiction" }] }, { name: "Due", type: "date" }, { name: "Status", type: "status", options: [{ name: "Done", group: "done" }] }] });
    const [rating, genre, , status] = page.database!.properties;
    // Titles fold ASCII case only and sort by code point, the core's UTF-8
    // byte order: 🚀 (U+1F680) after ﬁ (U+FB01), which UTF-16 would reverse.
    for (const title of ["日本語メモ", "Émile", "plain", "🚀 launch", "ﬁle"]) mockCreatePage({ title, parentId: page.note.id });
    const first = mockQueryDatabase({ databaseId: page.note.id, sorts: [{ property: "title", direction: "desc" }], limit: 1 });
    const rest = mockQueryDatabase({ databaseId: page.note.id, sorts: [{ property: "title", direction: "desc" }], limit: 5, cursor: first.nextCursor });
    expect([...first.rows, ...rest.rows].map(row => row.note.title)).toEqual(["🚀 launch", "ﬁle", "日本語メモ", "Émile", "plain"]);

    const refuse = (params: Partial<Parameters<typeof mockQueryDatabase>[0]>, message: string) =>
      expect(() => mockQueryDatabase({ databaseId: page.note.id, ...params }), message).toThrow(message);
    refuse({ filter: { property: rating.id, op: "eq", value: "5" } }, "takes a number");
    refuse({ filter: { property: genre.id, op: "is", value: "ghost" } }, "choose one of Fiction");
    refuse({ filter: { property: genre.id, op: "isAnyOf", value: genre.options![0].id } }, "takes a list");
    refuse({ filter: { property: status.id, op: "inGroup", value: "Done" } }, "inGroup takes one of");
    refuse({ filter: { property: "kind", op: "is", value: "essay" } }, "choose one of note");
    refuse({ filter: { property: "title", op: "contains", value: 3 } }, "takes text");
    refuse({ filter: { and: [], property: "x" } }, "holds only its list");
    refuse({ filter: { property: "title", op: "isEmpty", extra: 1 } }, "not extra");
    refuse({ groupBy: { property: page.database!.properties[2].id, dateBucket: "year" } }, "day, week or month");
    refuse({ today: "11/04/2026" }, "YYYY-MM-DD");

    // A page moved into the database takes cells in the same update.
    const loose = mockCreatePage({ title: "Loose" });
    mockUpdateNote({ id: loose.id, parentId: page.note.id, values: { [rating.id]: 3 } });
    expect(mockReadPage(loose.id).values).toEqual({ [rating.id]: 3 });
    // A deleted property's cell is not a row value.
    mockUpdateSchema({ id: page.note.id, changes: [{ op: "delete", propertyId: rating.id }] });
    const rows = mockQueryDatabase({ databaseId: page.note.id, groupBy: { property: genre.id } });
    expect(rows.rows.every(row => !(rating.id in row.values))).toBe(true);
    expect(rows.groups!.flatMap(group => group.rows).every(row => !(rating.id in row.values))).toBe(true);
  });
});

