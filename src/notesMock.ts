// In-memory Bridge Notes for `bun run dev` and component tests: the same
// shapes the core returns, a seq history per page so stale saves merge, and
// fake Structure and Turn into tasks passes that finish after a short delay and
// announce `notes/page_changed` the way the core does.

import { rebaseDraft } from "./notesDraft";
import type { CreatePageParams, EditPageParams, ListNotesParams, NoteJobQueued, SaveNotesSettingsParams, SplitItemParams, StructureNoteParams, UpdateNoteParams } from "./protocol/generated/protocol";
import type { Note, NoteEditResult, NoteJob, NotePage, NotePageChangedPayload, NotesList, NotesSettings, StructuredView } from "./types";

const ROOT = "/Users/you/Library/Application Support/Bridge/notes";
const PERSON = "person";

interface MockPage {
  note: Note;
  /** The body at each seq: `history[seq - 1]`. */
  history: string[];
  structured?: { view: Omit<StructuredView, "stale">; body: string };
  deleted: boolean;
}

let pages: MockPage[] = [];
let jobs: NoteJob[] = [];
let settings: NotesSettings = { autoStructure: false, root: ROOT };
let passDelay = 700;
let counter = 0;
const listeners = new Set<(payload: NotePageChangedPayload) => void>();

const stamp = () => new Date(Date.now() + counter).toISOString();
const nextId = (prefix: string) => `${prefix}-${(++counter).toString(36).padStart(4, "0")}`;
const clone = <T,>(value: T): T => structuredClone(value);

function announce(page: MockPage, actor: string) {
  const payload = { pageId: page.note.id, actor, seq: page.note.seq };
  for (const listener of [...listeners]) listener(payload);
}

function live(id: string): MockPage {
  const page = pages.find(candidate => candidate.note.id === id && !candidate.deleted);
  if (!page) throw new Error("no such note");
  return page;
}

/** Commit a body and properties as one new seq. */
function commit(page: MockPage, body: string, actor: string, props: Partial<Note> = {}) {
  page.history.push(body);
  page.note = { ...page.note, ...props, seq: page.history.length, preview: body.slice(0, 280), lastActor: actor, updatedAt: stamp() };
  announce(page, actor);
}

function make(body: string, actor: string, props: Partial<Note>): MockPage {
  const now = stamp();
  const page: MockPage = {
    note: {
      id: nextId("note"), spaceId: "personal", kind: "note", state: "inbox", tags: [], sessionIds: [], title: null,
      createdAt: now, updatedAt: now, createdBy: actor, lastActor: actor, preview: body.slice(0, 280), seq: 1,
      path: `${ROOT}/note-${counter}.md`, parentId: null, sourceExcerpt: null, sourceId: null, workspaceId: null,
      ...props,
    },
    history: [body],
    deleted: false,
  };
  pages.unshift(page);
  announce(page, actor);
  return page;
}

function seed() {
  pages = [];
  jobs = [];
  settings = { autoStructure: false, root: ROOT };
  make(
    "# Local-first sync libraries\n\nThree worth a look, from a quick survey:\n\n1. **Yjs**: CRDTs with bindings for every editor; the most used.\n2. **Automerge**: JSON-like documents, a Rust core, strong history.\n3. **Electric SQL**: Postgres sync into local SQLite.\n\nYjs fits a page editor best today.\n",
    "agent:claude:session-1w",
    { title: "Local-first sync libraries", state: "open", tags: ["research"] },
  );
  make("need to fix the onbaording flow, ppl drop at step 3\n- add a skip button\n- shorter copy on the welcome screen\nalso: pricing page idea, monthly vs yearly toggle", PERSON, {});
}
seed();

/** Back to the seeded notes. Tests call this between cases. */
export function resetMockNotes(): void {
  counter = 0;
  listeners.clear();
  passDelay = 700;
  seed();
}

/** How long a fake pass takes, so tests can make it instant. */
export function setMockNotesPassDelay(ms: number): void {
  passDelay = ms;
}

export function onMockNotesPageChanged(handler: (payload: NotePageChangedPayload) => void): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

/** An agent appends to a page (or writes a new one), the way a chat's notes
 *  tools would. */
export function mockNotesAgentWrite(pageId: string | null, text: string, sessionId = "session-1w", harness = "claude"): Note {
  const actor = `agent:${harness}:${sessionId}`;
  if (!pageId) return clone(make(text, actor, { state: "open" }).note);
  const page = live(pageId);
  const body = page.history[page.history.length - 1];
  commit(page, `${body}${body.endsWith("\n") || !body ? "" : "\n"}${text}`, actor);
  return clone(page.note);
}

export function mockListNotes(params: ListNotesParams = {}): NotesList {
  const query = params.query?.trim().toLowerCase();
  const notes = pages
    .filter(page => !page.deleted)
    .filter(page => params.includeArchived || page.note.state !== "archived")
    .filter(page => !params.kind || page.note.kind === params.kind)
    .filter(page => !params.state || page.note.state === params.state)
    .filter(page => params.parentId === undefined || params.parentId === null || page.note.parentId === params.parentId)
    .filter(page => !params.tags?.length || params.tags.every(tag => page.note.tags.includes(tag)))
    .filter(page => !query || `${page.note.title ?? ""}\n${page.history[page.history.length - 1]}`.toLowerCase().includes(query))
    .map(page => clone(page.note));
  return { notes, jobs: clone(jobs), root: settings.root };
}

export function mockReadPage(id: string): NotePage {
  const page = live(id);
  const markdown = page.history[page.history.length - 1];
  const structured = page.structured ? { ...clone(page.structured.view), stale: page.structured.body !== markdown } : null;
  const tasks = pages.filter(candidate => !candidate.deleted && candidate.note.parentId === id && candidate.note.kind === "task").map(candidate => clone(candidate.note)).reverse();
  return { note: clone(page.note), markdown, seq: page.note.seq, structured, tasks };
}

export function mockCaptureNote(body: string): Note {
  if (!body.trim()) throw new Error("a capture needs some text");
  const page = make(body, PERSON, {});
  if (settings.autoStructure) mockStructureNote({ id: page.note.id });
  return clone(page.note);
}

export function mockCreatePage(params: CreatePageParams): Note {
  return clone(make(params.content ?? "", PERSON, {
    title: params.title ?? null,
    kind: params.kind ?? "note",
    tags: params.tags ?? [],
    parentId: params.parentId ?? null,
    workspaceId: params.workspaceId ?? null,
    state: params.parentId ? "open" : "inbox",
  }).note);
}

export function mockEditPage(params: EditPageParams): NoteEditResult {
  const page = live(params.id);
  const current = page.history[page.history.length - 1];
  let next = current;
  if (params.text !== undefined && params.text !== null) {
    const base = params.baseSeq ? page.history[params.baseSeq - 1] : undefined;
    // The core merges a stale save as CRDT ops; replaying the person's edit
    // onto the current body is the same outcome for the cases the UI makes.
    next = base === undefined || base === current ? params.text : rebaseDraft(base, params.text, current).text;
  }
  for (const edit of params.edits ?? []) {
    if (next.split(edit.oldStr).length !== 2) throw new Error(`"${edit.oldStr}" must match exactly once`);
    next = next.replace(edit.oldStr, edit.newStr);
  }
  if (params.append) next = params.append.position === "start" ? `${params.append.content}${next}` : `${next}${params.append.content}`;
  if (next !== current) commit(page, next, PERSON);
  return { note: clone(page.note), seq: page.note.seq, warnings: [] };
}

export function mockUpdateNote(params: UpdateNoteParams): Note {
  const page = live(params.id);
  const props: Partial<Note> = {};
  if (params.title !== undefined && params.title !== null) props.title = params.title.trim() || null;
  if (params.kind) props.kind = params.kind;
  if (params.tags) props.tags = params.tags;
  if (params.state) props.state = params.state;
  if (params.workspaceId) props.workspaceId = params.workspaceId;
  if (params.clearWorkspace) props.workspaceId = null;
  if (params.addSessionId && !page.note.sessionIds.includes(params.addSessionId)) props.sessionIds = [...page.note.sessionIds, params.addSessionId];
  commit(page, page.history[page.history.length - 1], PERSON, props);
  return clone(page.note);
}

export function mockDeleteNote(id: string): void {
  const page = live(id);
  for (const task of pages.filter(candidate => candidate.note.parentId === id)) task.deleted = true;
  page.deleted = true;
  announce(page, PERSON);
}

function queue(kind: "structure" | "tasks", params: StructureNoteParams, run: (page: MockPage) => string | null): NoteJobQueued {
  const page = live(params.id);
  const job: NoteJob = { id: nextId("job"), pageId: page.note.id, kind, status: "pending", harness: params.harness ?? "claude", model: params.model ?? null, detail: null, createdAt: stamp(), updatedAt: stamp() };
  jobs = [job, ...jobs];
  setTimeout(() => {
    const settle = (status: string, detail: string | null) => { jobs = jobs.map(item => item.id === job.id ? { ...item, status, detail, updatedAt: stamp() } : item); };
    if (page.deleted) { settle("failed", "the note was deleted"); return; }
    const failure = run(page);
    settle(failure ? "failed" : "done", failure);
  }, passDelay);
  return { jobId: job.id };
}

const KIND_HINTS: [RegExp, string][] = [[/\?\s*$/m, "question"], [/\b(need to|todo|fix|should)\b/i, "task"], [/\b(idea|what if|maybe)\b/i, "idea"]];
const kindOf = (text: string) => KIND_HINTS.find(([pattern]) => pattern.test(text))?.[1] ?? "note";
const lineTitle = (line: string) => {
  const clean = line.replace(/^[-*#\d.\s]+/, "").replace(/\*\*/g, "").trim();
  return clean.length > 60 ? `${clean.slice(0, 59)}…` : clean;
};

export function mockStructureNote(params: StructureNoteParams): NoteJobQueued {
  return queue("structure", params, page => {
    const body = page.history[page.history.length - 1];
    const lines = body.split("\n").map(line => line.trim()).filter(Boolean);
    if (!lines.length) return "There is nothing to structure yet.";
    page.structured = {
      body,
      view: {
        title: lineTitle(lines[0]) || "Untitled",
        kind: kindOf(body),
        tags: page.note.tags.length ? page.note.tags : ["inbox"],
        summary: `${lines.length} thought${lines.length === 1 ? "" : "s"}, starting with "${lineTitle(lines[0])}".`,
        items: lines.slice(0, 12).map(line => ({ excerpt: line, title: lineTitle(line) || line, kind: kindOf(line) })),
        harness: params.harness ?? "claude",
        model: params.model ?? "claude-haiku-4-5",
        createdAt: stamp(),
        workspaceId: null,
      },
    };
    announce(page, `model:structure:${params.model ?? "claude-haiku-4-5"}`);
    return null;
  });
}

export function mockExtractTasks(params: StructureNoteParams): NoteJobQueued {
  return queue("tasks", params, page => {
    const body = page.history[page.history.length - 1];
    const existing = pages.filter(candidate => !candidate.deleted && candidate.note.parentId === page.note.id).map(candidate => candidate.note.sourceExcerpt);
    const found = body.split("\n").map(line => line.trim()).filter(line => /^- /.test(line) || /\b(need to|todo)\b/i.test(line));
    for (const excerpt of found) {
      if (existing.includes(excerpt)) continue;
      const title = lineTitle(excerpt);
      make(`${title}.\n\nDone when: ${title.toLowerCase()} is shipped and checked.`, `model:tasks:${params.model ?? "claude-sonnet-5-5"}`, {
        title, kind: "task", state: "open", parentId: page.note.id, sourceExcerpt: excerpt, workspaceId: page.note.workspaceId ?? null,
      });
    }
    return null;
  });
}

export function mockSplitItem(params: SplitItemParams): Note {
  const page = live(params.id);
  const item = page.structured?.view.items[params.itemIndex];
  if (!item) throw new Error("no such item");
  return clone(make(item.excerpt, PERSON, { title: item.title, kind: item.kind, sourceId: page.note.id }).note);
}

export function mockNotesSettings(): NotesSettings {
  return clone(settings);
}

export function mockSaveNotesSettings(params: SaveNotesSettingsParams): NotesSettings {
  settings = { autoStructure: params.autoStructure ?? settings.autoStructure, root: params.root?.trim() || settings.root };
  return clone(settings);
}
