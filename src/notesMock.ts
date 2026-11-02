// In-memory Bridge Notes for `bun run dev` and component tests: the same
// shapes the core returns, a seq history per page so stale saves merge, and
// fake Structure and Turn into tasks passes that stream `notes/pass_progress`,
// finish after a short delay and announce `notes/page_changed` the way the
// core does, and images kept by path.

import { rebaseDraft, noteDisplayTitle } from "./notesDraft";
import type { CreatePageParams, EditPageParams, ListNotesParams, NoteBacklink, NoteJobQueued, SaveNotesSettingsParams, SplitItemParams, StructureNoteParams, UpdateNoteParams } from "./protocol/generated/protocol";
import type { Note, NoteAsset, NoteAssetData, NoteAssetPart, NoteEditResult, NoteHistory, NoteJob, NotePage, NotePageChangedPayload, NotePassProgressPayload, NotesList, NotesSettings, NoteVersionPage, StructuredView } from "./types";

const ROOT = "/Users/you/Library/Application Support/Bridge/notes";
const PERSON = "person";

interface MockPage {
  note: Note;
  /** The body at each seq: `history[seq - 1]`. */
  history: string[];
  /** Who wrote each seq, when, and the title then. */
  log: { actor: string; at: string; title: string | null }[];
  structured?: { view: Omit<StructuredView, "stale">; body: string };
  deleted: boolean;
}

let pages: MockPage[] = [];
let jobs: NoteJob[] = [];
let settings: NotesSettings = { autoStructure: false, root: ROOT };
let passDelay = 700;
let counter = 0;
const listeners = new Set<(payload: NotePageChangedPayload) => void>();
const progressListeners = new Set<(payload: NotePassProgressPayload) => void>();
let uploads = new Map<string, string>();
let assets = new Map<string, NoteAssetData>();

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
  page.log.push({ actor, at: page.note.updatedAt, title: page.note.title ?? null });
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
    log: [],
    deleted: false,
  };
  page.log.push({ actor, at: now, title: page.note.title ?? null });
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
  uploads = new Map();
  assets = new Map();
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

/** Every word of the query matches the start of a word in the title or body,
 *  the way the core's FTS prefix queries read (`runs` finds `running`). */
function searchHaystack(needle: string, text: string): boolean {
  const words = needle.toLowerCase().trim().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const tokens = text.toLowerCase().split(/[^\p{L}\p{N}]+/u);
  return words.every(word => tokens.some(token => token.startsWith(word)));
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
    .filter(page => !query || searchHaystack(query, `${page.note.title ?? ""}\n${page.history[page.history.length - 1]}`))
    .map(page => clone(page.note));
  return { notes, jobs: clone(jobs), root: settings.root };
}

export function mockReadPage(id: string): NotePage {
  const page = live(id);
  const markdown = page.history[page.history.length - 1];
  const structured = page.structured ? { ...clone(page.structured.view), stale: page.structured.body !== markdown } : null;
  const tasks = pages.filter(candidate => !candidate.deleted && candidate.note.parentId === id && candidate.note.kind === "task").map(candidate => clone(candidate.note)).reverse();
  return { note: clone(page.note), markdown, seq: page.note.seq, structured, tasks, backlinks: backlinksOf(id) };
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
    if (!params.allowDeletingChildren) {
      const dropped = (current.match(/<(?:page|database)\b[^>]*>/g) ?? []).filter(tag => !next.includes(tag));
      if (dropped.length) throw new Error(`this edit would delete ${dropped.length} child page or database tag(s): ${dropped.join(", ")}. Ask the person first, then retry with allowDeletingChildren.`);
    }
    if (next !== current) commit(page, next, PERSON);
  return { note: clone(page.note), seq: page.note.seq, warnings: [] };
}

export function mockUpdateNote(params: UpdateNoteParams): Note {
  const page = live(params.id);
  const props: Partial<Note> = {};
  if (params.clearParent && params.parentId != null) throw new Error("parentId and clearParent are mutually exclusive");
  if (params.parentId != null) {
    let parent: string | null = params.parentId;
    const seen = new Set<string>([params.id]);
    let depth = 0;
    while (parent !== null) {
      if (seen.has(parent) || depth++ >= 64) throw new Error("a page cannot be its own ancestor");
      seen.add(parent);
      const candidate: Note = live(parent).note;
      if (candidate.kind === "task") throw new Error("a task cannot be a parent page");
      parent = candidate.parentId ?? null;
    }
    props.parentId = params.parentId;
  }
  if (params.clearParent) props.parentId = null;
  if ((params.clearParent || params.parentId != null) && page.note.state === "inbox" && !params.state) props.state = "open";
  const previousTitle = page.note.title ?? null;
  if (params.title !== undefined && params.title !== null) props.title = params.title.trim() || null;
  if (params.kind) props.kind = params.kind;
  if (params.tags) props.tags = params.tags;
  if (params.state) props.state = params.state;
  if (params.workspaceId) props.workspaceId = params.workspaceId;
  if (params.clearWorkspace) props.workspaceId = null;
  if (params.addSessionId && !page.note.sessionIds.includes(params.addSessionId)) props.sessionIds = [...page.note.sessionIds, params.addSessionId];
  // A title the links followed takes them with it: decide who names the page
  // before the rename lands, so a duplicate title cannot claim a link.
  const newTitle = props.title ?? null;
  const renaming = previousTitle != null && newTitle != null && newTitle !== "" && newTitle !== previousTitle;
  const renameRequests = renaming ? renameRecipients(page.note.id, previousTitle) : [];
  commit(page, page.history[page.history.length - 1], PERSON, props);
  for (const from of renameRequests) {
    const source = live(from);
    const body = source.history[source.history.length - 1];
    const rewritten = renameBody(body, page.note.id, previousTitle, newTitle!, true);
    if (rewritten !== body) commit(source, rewritten, PERSON);
  }
  return clone(page.note);
}

export function mockDeleteNote(id: string): void {
  const page = live(id);
  for (const task of pages.filter(candidate => candidate.note.parentId === id)) task.deleted = true;
  page.deleted = true;
  announce(page, PERSON);
}

// The link graph, the mock's side of the core's `links.rs`: `[[Target]]`, with
// an optional `| alias`, and `<page url="bridge://p/id">` tags resolve between
// pages, Markdown stays authoritative, and a rename rewrites only the link
// byte ranges never the words around them.

interface WikiSpan { start: number; end: number; target: string; alias: string | null; kind: "wiki" | "page"; opening: string }

const linkKey = (title: string) => title.trim().toLowerCase();
const escapeLinkText = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/\r/g, "&#13;").replace(/\n/g, "&#10;");
const unescapeLinkText = (text: string) => text.replace(/&(?:amp|lt|gt|quot|13|10);/g, character => ({ "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": "\"", "&#13;": "\r", "&#10;": "\n" })[character] ?? character);

/** A line that starts a list item, so indented code never starts inside one. */
function listMarker(text: string): boolean {
  const digits = /^[0-9]*/.exec(text)![0].length;
  if (/^[+*-]/.test(text)) return text.length === 1 || /^[ \t\r\n]/.test(text.slice(1));
  if (digits >= 1 && digits <= 9) {
    const after = text.slice(digits + 1);
    if (text.charAt(digits) === "." || text.charAt(digits) === ")") return after.length === 0 || /^[ \t\r\n]/.test(after);
  }
  return false;
}

/** Every line of code masked with spaces, keeping byte offsets so link ranges
 *  line up with the file (the mock's `outside_code`). */
function codeMask(body: string): string {
  const lines = body.split("\n");
  const masked: string[] = [];
  let fence: { char: string; length: number } | null = null;
  let blockStart = true;
  let inCode = false;
  let inList = false;
  lines.forEach((line, index) => {
    const width = line.length - line.trimStart().length;
    const content = line.slice(width);
    const blank = content.trim() === "";
    const marker = content.charAt(0);
    let count = 0;
    while (count < content.length && content.charAt(count) === marker) count += 1;
    const fenceLine = width <= 3 && (marker === "`" || marker === "~") && count >= 3;
    let code: boolean;
    if (fence) {
      if (fenceLine && marker === fence.char && count >= fence.length && !content.slice(count).trim()) fence = null;
      code = true;
    } else if (fenceLine && (marker === "~" || !content.slice(count).includes("`"))) {
      fence = { char: marker, length: count };
      code = true;
    } else {
      code = !blank && width >= 4 && (blockStart || inCode) && !inList;
    }
    if (code) masked.push(" ".repeat(line.length));
    else {
      masked.push(line);
      if (!blank && width < 4) inList = listMarker(content);
    }
    if (index < lines.length - 1) masked.push("\n");
    inCode = code && fence === null && !fenceLine;
    blockStart = blank || fenceLine || content.startsWith("#");
  });
  return masked.join("").replace(/`[^`\n]*`/g, span => " ".repeat(span.length));
}

/** Every `[[…]]` and `<page …>` the body holds, with the byte ranges they sit
 *  at, the way `links.rs::parse` finds them. Code is never a link. */
function linksOf(body: string): WikiSpan[] {
  const masked = codeMask(body);
  const found: WikiSpan[] = [];
  const wiki = /\[\[([^\]\r\n]+?)\]\]/g;
  for (const match of body.matchAll(wiki)) {
    const start = match.index!;
    // The mask turned code into spaces, so a match whose slice is now blank
    // sat inside code.
    if (masked.slice(start, start + match[0].length).trim() === "") continue;
    const bar = match[1].indexOf("|");
    const target = (bar >= 0 ? match[1].slice(0, bar) : match[1]).trim();
    const alias = bar >= 0 ? match[1].slice(bar + 1).trim() || null : null;
    if (!target) continue;
    found.push({ start, end: start + match[0].length, target, alias, kind: "wiki", opening: "" });
  }
  const pageTag = /(<page\s+url="bridge:\/\/p\/([^"<>\s]+)"\s*>)([^\r\n]*?)<\/page>/g;
  for (const match of body.matchAll(pageTag)) {
    const start = match.index!;
    if (masked.slice(start, start + match[0].length).trim() === "") continue;
    if (!match[2].trim()) continue;
    found.push({ start, end: start + match[0].length, target: match[2], alias: null, kind: "page", opening: match[1] });
  }
  return found;
}

/** `[[target]]` becomes the source's text without the double brackets, the way
 *  the core quotes a link's line. */
function excerptOf(body: string, link: WikiSpan): string {
  const start = body.lastIndexOf("\n", link.start - 1) < 0 ? 0 : body.lastIndexOf("\n", link.start - 1) + 1;
  const end = body.indexOf("\n", link.end) < 0 ? body.length : body.indexOf("\n", link.end);
  const line = body
    .slice(start, end)
    .replace(/\[\[([^\]\r\n]+?)\]\]/g, (_, held: string) => (held.includes("|") ? held.slice(held.indexOf("|") + 1) : held))
    .replace(/<page\s+url="bridge:\/\/p\/[^"<>\s]+"\s*>([^\r\n]*?)<\/page>/g, (_, held: string) => unescapeLinkText(held));
  return [...line.trim()].filter(character => character.charCodeAt(0) > 31).slice(0, 200).join("");
}

/** Every backlink page `id` has, newest change first. */
function backlinksOf(id: string): NoteBacklink[] {
  const target = live(id).note;
  const title = target.title?.trim().toLowerCase() ?? null;
  const rows: NoteBacklink[] = [];
  for (const page of [...pages].sort((a, b) => a.note.createdAt.localeCompare(b.note.createdAt))) {
    if (page.deleted || page.note.id === id) continue;
    const body = page.history[page.history.length - 1];
    for (const link of linksOf(body)) {
      const linked = link.kind === "page"
        ? link.target === id
        : title !== null && linkKey(link.target) === title;
      if (!linked) continue;
      rows.push({
        pageId: page.note.id,
        title: noteDisplayTitle({ title: page.note.title, preview: page.note.preview }),
        kind: link.kind,
        excerpt: excerptOf(body, link),
        updatedAt: page.note.updatedAt,
      });
    }
  }
  return rows.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || a.pageId.localeCompare(b.pageId) || a.kind.localeCompare(b.kind));
}

/** The pages that point at `renamed` with a wiki link through its old title:
 *  the first page with that title is where `[[old]]` resolves. */
function renameRecipients(id: string, oldTitle: string): string[] {
  const key = linkKey(oldTitle);
  const sources = pages.filter(page => !page.deleted && linksOf(page.history[page.history.length - 1]).some(link => link.kind === "wiki" && linkKey(link.target) === key));
  const holder = pages.filter(page => !page.deleted && page.note.title != null && linkKey(page.note.title!) === key)
    .sort((a, b) => a.note.createdAt.localeCompare(b.note.createdAt) || a.note.id.localeCompare(b.note.id))[0];
  return holder && holder.note.id === id ? sources.map(source => source.note.id) : [];
}

/** What a rename does to one body: wiki links that resolved to this page move
 *  with it (alias kept), page tags follow the id, and a title the wiki grammar
 *  cannot hold becomes a page tag. */
function renameBody(body: string, id: string, oldTitle: string, newTitle: string, wiki: boolean): string {
  let result = body;
  const safe = !newTitle.includes("[") && !newTitle.includes("]") && !newTitle.includes("|") && !/[\r\n]/.test(newTitle);
  for (const link of linksOf(body).reverse()) {
    let replacement: string | null = null;
    if (wiki && link.kind === "wiki" && linkKey(link.target) === linkKey(oldTitle)) {
      replacement = safe
        ? link.alias ? `[[${newTitle}|${link.alias}]]` : `[[${newTitle}]]`
        : `<page url="bridge://p/${id}">${escapeLinkText(link.alias ?? newTitle)}</page>`;
    } else if (link.kind === "page" && link.target === id) {
      replacement = `${link.opening}${escapeLinkText(newTitle)}</page>`;
    }
    if (replacement) result = result.slice(0, link.start) + replacement + result.slice(link.end);
  }
  return result;
}

export function onMockNotesPassProgress(handler: (payload: NotePassProgressPayload) => void): () => void {
  progressListeners.add(handler);
  return () => { progressListeners.delete(handler); };
}

/** Queue a pass. `reply` is the model's whole reply, streamed in thirds
 *  before `run` stores the result. */
function queue(kind: "structure" | "tasks", params: StructureNoteParams, reply: (page: MockPage) => string, run: (page: MockPage) => string | null): NoteJobQueued {
  const page = live(params.id);
  const job: NoteJob = { id: nextId("job"), pageId: page.note.id, kind, status: "pending", harness: params.harness ?? "claude", model: params.model ?? null, detail: null, createdAt: stamp(), updatedAt: stamp() };
  jobs = [job, ...jobs];
  for (const fraction of [1 / 3, 2 / 3]) {
    setTimeout(() => {
      if (page.deleted) return;
      jobs = jobs.map(item => item.id === job.id && item.status === "pending" ? { ...item, status: "running" } : item);
      const text = reply(page);
      const payload = { jobId: job.id, pageId: page.note.id, kind, text: text.slice(0, Math.floor(text.length * fraction)) };
      for (const listener of [...progressListeners]) listener(payload);
    }, passDelay * fraction);
  }
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

function structureOf(page: MockPage) {
  const body = page.history[page.history.length - 1];
  const lines = body.split("\n").map(line => line.trim()).filter(Boolean);
  if (!lines.length) return null;
  return {
    title: lineTitle(lines[0]) || "Untitled",
    kind: kindOf(body),
    tags: page.note.tags.length ? page.note.tags : ["inbox"],
    summary: `${lines.length} thought${lines.length === 1 ? "" : "s"}, starting with "${lineTitle(lines[0])}".`,
    items: lines.slice(0, 12).map(line => ({ excerpt: line, title: lineTitle(line) || line, kind: kindOf(line) })),
  };
}

export function mockStructureNote(params: StructureNoteParams): NoteJobQueued {
  const reply = (page: MockPage) => JSON.stringify({ ...structureOf(page), project: null });
  return queue("structure", params, reply, page => {
    const view = structureOf(page);
    if (!view) return "There is nothing to structure yet.";
    page.structured = {
      body: page.history[page.history.length - 1],
      view: { ...view, harness: params.harness ?? "claude", model: params.model ?? "claude-haiku-4-5", createdAt: stamp(), workspaceId: null },
    };
    announce(page, `model:structure:${params.model ?? "claude-haiku-4-5"}`);
    return null;
  });
}

const tasksOf = (page: MockPage) => page.history[page.history.length - 1].split("\n").map(line => line.trim()).filter(line => /^- /.test(line) || /\b(need to|todo)\b/i.test(line));

export function mockExtractTasks(params: StructureNoteParams): NoteJobQueued {
  const reply = (page: MockPage) => JSON.stringify({ tasks: tasksOf(page).map(excerpt => ({ excerpt, title: lineTitle(excerpt), detail: "", doneWhen: "", project: null })) });
  return queue("tasks", params, reply, page => {
    const existing = pages.filter(candidate => !candidate.deleted && candidate.note.parentId === page.note.id).map(candidate => candidate.note.sourceExcerpt);
    for (const excerpt of tasksOf(page)) {
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

export function mockPageHistory(id: string): NoteHistory {
  const page = live(id);
  const versions = page.history.map((body, index) => {
    const before = index ? page.history[index - 1] : "";
    let head = 0;
    while (head < before.length && head < body.length && before[head] === body[head]) head += 1;
    let tail = 0;
    while (tail < before.length - head && tail < body.length - head && before[before.length - 1 - tail] === body[body.length - 1 - tail]) tail += 1;
    const entry = page.log[index];
    return { seq: index + 1, actor: entry.actor, at: entry.at, bodyChanged: index === 0 ? body.length > 0 : body !== before, added: body.length - head - tail, removed: before.length - head - tail, title: entry.title };
  });
  return { versions: versions.reverse() };
}

export function mockReadVersion(id: string, seq: number): NoteVersionPage {
  const page = live(id);
  const markdown = page.history[seq - 1];
  if (markdown === undefined) throw new Error("no such version");
  const entry = page.log[seq - 1];
  return { seq, actor: entry.actor, at: entry.at, title: entry.title, markdown };
}

export function mockRestoreVersion(id: string, seq: number): NoteEditResult {
  const page = live(id);
  const words = mockReadVersion(id, seq).markdown;
  const changed = words !== page.history[page.history.length - 1];
  if (changed) commit(page, words, PERSON);
  return { note: clone(page.note), seq: page.note.seq, warnings: changed ? [] : ["nothing changed"] };
}

/** Back-date a version, so a test can lay out a history across time. */
export function mockSetVersionTime(id: string, seq: number, at: string): void {
  live(id).log[seq - 1].at = at;
}

export function mockNotesSettings(): NotesSettings {
  return clone(settings);
}

export function mockSaveNotesSettings(params: SaveNotesSettingsParams): NotesSettings {
  settings = { autoStructure: params.autoStructure ?? settings.autoStructure, root: params.root?.trim() || settings.root };
  return clone(settings);
}

/** A stand-in for the core's content hash: 16 hex characters. */
function mockHash(data: string): string {
  let a = 0x811c9dc5;
  let b = 0x01000193;
  for (let index = 0; index < data.length; index += 1) {
    a = Math.imul(a ^ data.charCodeAt(index), 16777619) >>> 0;
    b = Math.imul(b ^ data.charCodeAt(index), 2246822519) >>> 0;
  }
  return a.toString(16).padStart(8, "0") + b.toString(16).padStart(8, "0");
}

const MOCK_EXT: Record<string, string> = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp", "image/svg+xml": "svg" };

export function mockPutAssetPart(params: { uploadId?: string | null; data: string }): NoteAssetPart {
  const uploadId = params.uploadId ?? nextId("upload");
  if (params.uploadId && !uploads.has(params.uploadId)) throw new Error("that upload expired or never started; send it again");
  // Parts are base64 of whole 3-byte groups except the last, so they join.
  const data = (uploads.get(uploadId) ?? "") + params.data;
  uploads.set(uploadId, data);
  return { uploadId, size: Math.floor((data.replace(/=+$/, "").length * 3) / 4) };
}

export function mockFinishAsset(params: { uploadId: string; mime: string }): NoteAsset {
  const data = uploads.get(params.uploadId);
  if (data === undefined) throw new Error("that upload expired or never started; send it again");
  const ext = MOCK_EXT[params.mime];
  if (!ext) throw new Error(`${params.mime} is not an image Bridge stores (PNG, JPEG, GIF, WebP or SVG)`);
  uploads.delete(params.uploadId);
  const path = `assets/${mockHash(data)}.${ext}`;
  assets.set(path, { path, mime: params.mime, data });
  return { path, mime: params.mime, size: Math.floor((data.replace(/=+$/, "").length * 3) / 4) };
}

export function mockReadAsset(path: string): NoteAssetData {
  const asset = assets.get(path);
  if (!asset) throw new Error(`no asset at ${path}`);
  return { ...asset };
}
