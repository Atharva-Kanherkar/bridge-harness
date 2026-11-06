// In-memory Bridge Notes for `bun run dev` and component tests: the same
// shapes the core returns, a seq history per page so stale saves merge, and
// fake Structure and Turn into tasks passes that stream `notes/pass_progress`,
// finish after a short delay and announce `notes/page_changed` the way the
// core does, and images kept by path.

import { rebaseDraft, noteDisplayTitle } from "./notesDraft";
import type { CreateDatabaseParams, CreatePageParams, DatabaseGroup, DatabaseQueryResult, DatabaseRow, DeleteViewParams, EditPageParams, ListNotesParams, NoteBacklink, NoteDatabase, NoteDatabaseView, NoteJobQueued, NoteOption, NoteProperty, NoteSort, NoteViewInput, QueryDatabaseParams, SaveNotesSettingsParams, SaveViewParams, SplitItemParams, StructureNoteParams, UpdateNoteParams, UpdateSchemaParams } from "./protocol/generated/protocol";
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
  /** A database page's records: its scope, properties and views by id. */
  database?: MockDatabase;
  /** A row's cells by property id. */
  values?: Record<string, unknown>;
}

interface MockDatabase {
  scope: "children" | "space";
  properties: Record<string, NoteProperty>;
  views: Record<string, NoteDatabaseView>;
}

let pages: MockPage[] = [];
/** The database whose saved views are the Notes list's tabs. */
let systemDatabaseId = "";
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
  seedSystemDatabase();
  make(
    "# Local-first sync libraries\n\nThree worth a look, from a quick survey:\n\n1. **Yjs**: CRDTs with bindings for every editor; the most used.\n2. **Automerge**: JSON-like documents, a Rust core, strong history.\n3. **Electric SQL**: Postgres sync into local SQLite.\n\nYjs fits a page editor best today.\n",
    "agent:claude:session-1w",
    { title: "Local-first sync libraries", state: "open", tags: ["research"] },
  );
  make("need to fix the onbaording flow, ppl drop at step 3\n- add a skip button\n- shorter copy on the welcome screen\nalso: pricing page idea, monthly vs yearly toggle", PERSON, {});
  // `bun run dev` opens on a database to try; tests keep the two pages above.
  if (import.meta.env.MODE !== "test") seedMockReadingList();
}

/** "Reading list": ten rows across every property type, for `bun run dev`. */
export function seedMockReadingList(): string {
  const page = mockCreateDatabase({
    title: "Reading list",
    properties: [
      { name: "Status", type: "status", options: [{ name: "To read", group: "todo" }, { name: "Reading", group: "inProgress" }, { name: "Read", group: "done" }] },
      { name: "Rating", type: "number" },
      { name: "Genre", type: "select", options: [{ name: "Systems" }, { name: "Fiction" }, { name: "History" }] },
      { name: "Labels", type: "multiSelect", options: [{ name: "Short" }, { name: "Long" }, { name: "Gift" }] },
      { name: "Due", type: "date" },
      { name: "Owned", type: "checkbox" },
      { name: "Link", type: "url" },
      { name: "Note", type: "text" },
    ],
  });
  const id = page.note.id;
  const related = mockUpdateSchema({ id, changes: [{ op: "add", property: { name: "Pairs with", type: "relation", relation: id } }] });
  const prop = (name: string) => related.properties.find(property => property.name === name)!;
  const option = (name: string, choice: string) => prop(name).options!.find(item => item.name === choice)!.id;
  const books: [string, string, number | null, string | null, string[], string | null, boolean, string | null, string | null][] = [
    ["Designing Data-Intensive Applications", "Reading", 5, "Systems", ["Long"], "2026-11-20", true, "https://dataintensive.net", "Chapter 5 next"],
    ["Structure and Interpretation of Computer Programs", "To read", 4, "Systems", ["Long", "Gift"], "2026-12-01", false, null, null],
    ["The Left Hand of Darkness", "Read", 5, "Fiction", ["Short"], null, true, null, "Reread the ice crossing"],
    ["A Philosophy of Software Design", "Read", 4, "Systems", ["Short"], "2026-10-28", true, null, null],
    ["The Dawn of Everything", "To read", null, "History", ["Long"], "2027-01-15", false, null, null],
    ["Piranesi", "Reading", 4.5, "Fiction", ["Short", "Gift"], "2026-11-12", true, null, null],
    ["Crafting Interpreters", "To read", null, "Systems", [], null, false, "https://craftinginterpreters.com", null],
    ["SPQR", "Read", 3.5, "History", [], "2026-09-30", true, null, "Good on the late republic"],
    ["Klara and the Sun", "To read", null, "Fiction", ["Gift"], null, false, null, null],
    ["The Mythical Man-Month", "Read", 3, "Systems", ["Short"], null, true, null, null],
  ];
  const rows: string[] = [];
  for (const [title, status, rating, genre, labels, due, owned, link, note] of books) {
    const row = mockCreatePage({ title, parentId: id });
    rows.push(row.id);
    const values: Record<string, unknown> = { [prop("Status").id]: option("Status", status), [prop("Owned").id]: owned };
    if (rating !== null) values[prop("Rating").id] = rating;
    if (genre) values[prop("Genre").id] = option("Genre", genre);
    if (labels.length) values[prop("Labels").id] = labels.map(label => option("Labels", label));
    if (due) values[prop("Due").id] = { start: due };
    if (link) values[prop("Link").id] = link;
    if (note) values[prop("Note").id] = note;
    mockUpdateNote({ id: row.id, values });
  }
  mockUpdateNote({ id: rows[0], values: { [prop("Pairs with").id]: [rows[3]] } });
  return id;
}

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
  // No word to search for (only symbols): a literal, ASCII-folded match, as
  // the core falls back to.
  if (needle.trim() && !/[\p{L}\p{N}]/u.test(needle)) return asciiLower(text).includes(asciiLower(needle.trim()));
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
    .filter(page => page.note.id !== systemDatabaseId)
    .map(page => clone(page.note));
  return { notes, jobs: clone(jobs), root: settings.root, systemDatabaseId };
}

export function mockReadPage(id: string): NotePage {
  const page = live(id);
  const markdown = page.history[page.history.length - 1];
  const structured = page.structured ? { ...clone(page.structured.view), stale: page.structured.body !== markdown } : null;
  const tasks = pages.filter(candidate => !candidate.deleted && candidate.note.parentId === id && candidate.note.kind === "task").map(candidate => clone(candidate.note)).reverse();
  const read: NotePage = { note: clone(page.note), markdown, seq: page.note.seq, structured, tasks, backlinks: backlinksOf(id) };
  if (page.database) read.database = databaseOf(page);
  const parent = rowDatabase(page);
  if (parent) {
    read.databaseId = parent.note.id;
    read.values = Object.fromEntries(Object.entries(page.values ?? {}).filter(([prop]) => prop in parent.database!.properties).map(([prop, value]) => [prop, clone(value)]));
  }
  return read;
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
  if (page.note.id === systemDatabaseId) throw new Error("the Notes database is managed by Bridge");
  // Cells are checked before anything is written, against the database the
  // page will belong to after this update, as the core does.
  const targetParent = params.clearParent ? null : params.parentId ?? page.note.parentId ?? null;
  const cells = params.values ? checkCells(page, params.values, targetParent) : null;
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
  if (params.addSessionId && !page.note.sessionIds.includes(params.addSessionId)) {
    props.sessionIds = [...page.note.sessionIds, params.addSessionId];
    if (!params.state && page.note.kind === "task" && (page.note.state === "open" || page.note.state === "inbox")) props.state = "doing";
  }
  // A title the links followed takes them with it: decide who names the page
  // before the rename lands, so a duplicate title cannot claim a link.
  const newTitle = props.title ?? null;
  const rename = previousTitle && newTitle && newTitle !== previousTitle ? { from: previousTitle, to: newTitle } : null;
  const renameRequests = rename ? renameRecipients(page.note.id, rename.from) : [];
  if (cells) page.values = cells;
  commit(page, page.history[page.history.length - 1], PERSON, props);
  for (const request of renameRequests) {
    const source = live(request.id);
    const body = source.history[source.history.length - 1];
    const rewritten = renameBody(body, page.note.id, rename!.from, rename!.to, request.wiki);
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
  // In body order, like the core's one pattern, so a rewrite from the end
  // never shifts a link still to come.
  // One link per byte, the leftmost first, as one pattern would match.
  const kept: WikiSpan[] = [];
  for (const span of found.sort((a, b) => a.start - b.start)) if (!kept.length || span.start >= kept[kept.length - 1].end) kept.push(span);
  return kept;
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

/** The page `[[key]]` resolves to, as the core's `resolve_one` picks it:
 *  the oldest page going by that title. */
function wikiHolder(key: string): MockPage | undefined {
  return pages.filter(page => !page.deleted && page.note.id !== systemDatabaseId && linkKey(noteDisplayTitle(page.note)) === key)
    .sort((a, b) => a.note.createdAt.localeCompare(b.note.createdAt) || a.note.id.localeCompare(b.note.id))[0];
}

/** Every backlink page `id` has, newest change first. */
function backlinksOf(id: string): NoteBacklink[] {
  const rows: NoteBacklink[] = [];
  for (const page of [...pages].sort((a, b) => a.note.createdAt.localeCompare(b.note.createdAt))) {
    if (page.deleted || page.note.id === id) continue;
    const body = page.history[page.history.length - 1];
    for (const link of linksOf(body)) {
      const linked = link.kind === "page"
        ? link.target === id
        : wikiHolder(linkKey(link.target))?.note.id === id;
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

/** The pages a rename touches: every page tag pointing at `id`, and wiki
 *  links through the old title only when `id` is where `[[old]]` resolves
 *  (the oldest page with that title). */
function renameRecipients(id: string, oldTitle: string): { id: string; wiki: boolean }[] {
  const key = linkKey(oldTitle);
  const wiki = wikiHolder(key)?.note.id === id;
  return pages.filter(page => !page.deleted).flatMap(page => {
    const links = linksOf(page.history[page.history.length - 1]);
    const touched = links.some(link => (link.kind === "page" && link.target === id) || (wiki && link.kind === "wiki" && linkKey(link.target) === key));
    return touched ? [{ id: page.note.id, wiki }] : [];
  });
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
    const normalize = (text: string) => text.trim().replace(/\s+/g, " ");
    const existing = new Set(pages.filter(candidate => !candidate.deleted && candidate.note.kind === "task" && candidate.note.parentId === page.note.id).map(candidate => normalize(candidate.note.sourceExcerpt ?? "")));
    for (const excerpt of tasksOf(page)) {
      const key = normalize(excerpt);
      if (existing.has(key)) continue;
      existing.add(key);
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

// ---------------------------------------------------------------------------
// Databases: the mock's side of the core's `notes/query.rs` and the database
// methods. The rules match the core's exactly; the shared cases in
// `testing/fixtures/notes-query-cases.json` hold the two together.
// ---------------------------------------------------------------------------

const SYSTEM_PROPERTIES = ["title", "kind", "state", "tags", "project", "chats", "parent", "created", "updated", "createdBy"];
const KINDS = ["note", "idea", "task", "question", "database"];
const STATES = ["inbox", "open", "doing", "done", "archived"];
const STATUS_GROUPS = ["todo", "inProgress", "done"];
const PROPERTY_TYPES = ["text", "number", "select", "multiSelect", "date", "checkbox", "url", "relation", "status"];
const OPTION_TYPES = ["select", "multiSelect", "status"];

/** The seven Notes list views, as the core seeds them. */
function systemViews(): Record<string, NoteDatabaseView> {
  const condition = (property: string, op: string, value?: unknown) => (value === undefined ? { property, op } : { property, op, value });
  const notArchived = condition("state", "isNot", "archived");
  const notChildTask = { or: [condition("kind", "isNot", "task"), condition("parent", "isEmpty")] };
  const views: [string, unknown][] = [
    ["All", { and: [notArchived, notChildTask] }],
    ["Inbox", { and: [condition("state", "is", "inbox"), notChildTask] }],
    ["Pages", { and: [condition("kind", "isNot", "task"), notArchived] }],
    ["Tasks", { and: [condition("kind", "is", "task"), notArchived] }],
    ["Ideas", { and: [condition("kind", "is", "idea")] }],
    ["In progress with an agent", { and: [condition("state", "is", "doing"), condition("chats", "isNotEmpty")] }],
    ["Archived", { and: [condition("state", "is", "archived")] }],
  ];
  return Object.fromEntries(views.map(([name, filter], order) => {
    const id = nextId("view");
    return [id, { id, name, type: "list", order, filter }];
  }));
}

function seedSystemDatabase() {
  const now = stamp();
  const page: MockPage = {
    note: {
      id: nextId("database"), spaceId: "personal", kind: "database", state: "open", tags: [], sessionIds: [], title: "Notes",
      createdAt: now, updatedAt: now, createdBy: "system", lastActor: "system", preview: "", seq: 1,
      path: null, parentId: null, sourceExcerpt: null, sourceId: null, workspaceId: null,
    },
    history: [""],
    log: [{ actor: "system", at: now, title: "Notes" }],
    deleted: false,
    database: { scope: "space", properties: {}, views: systemViews() },
  };
  pages.push(page);
  systemDatabaseId = page.note.id;
}

const byOrder = <T extends { order: number; id: string }>(items: T[]) => [...items].sort((a, b) => a.order - b.order || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

function databaseOf(page: MockPage): NoteDatabase {
  const database = page.database!;
  return clone({ id: page.note.id, scope: database.scope, properties: byOrder(Object.values(database.properties)), views: byOrder(Object.values(database.views)) });
}

/** The live database a page is a row of. */
function rowDatabase(page: MockPage): MockPage | undefined {
  if (!page.note.parentId) return undefined;
  const parent = pages.find(candidate => candidate.note.id === page.note.parentId && !candidate.deleted);
  return parent?.database ? parent : undefined;
}

function liveDatabase(id: string): MockPage {
  const page = live(id);
  if (!page.database) throw new Error("this page is not a database");
  return page;
}

const fold = (text: string) => text.trim().toLowerCase();
const asciiLower = (text: string) => text.replace(/[A-Z]/g, character => character.toLowerCase());

function checkName(properties: Record<string, NoteProperty>, name: string, except?: string): string {
  const trimmed = name.trim();
  if (!trimmed) throw new Error("a property needs a name");
  if (SYSTEM_PROPERTIES.some(system => fold(system) === fold(trimmed)) || ["id", "database"].includes(fold(trimmed))) throw new Error(`${trimmed} is reserved; choose another name`);
  if (Object.values(properties).some(property => property.id !== except && fold(property.name) === fold(trimmed))) throw new Error(`this database already has a property named ${trimmed}`);
  return trimmed;
}

function newOptions(type: string, options: { name: string; color?: string | null; group?: string | null }[] | null | undefined): NoteOption[] {
  const built: NoteOption[] = [];
  for (const option of options ?? []) {
    const name = option.name.trim();
    if (!name || built.some(existing => fold(existing.name) === fold(name))) throw new Error(`an option needs a unique name: ${option.name}`);
    const group = type === "status" ? option.group ?? "todo" : null;
    if (group && !STATUS_GROUPS.includes(group)) throw new Error(`a status group must be one of ${STATUS_GROUPS.join(", ")}`);
    built.push({ id: nextId("option"), name, ...(option.color ? { color: option.color } : {}), ...(group ? { group } : {}) });
  }
  return built;
}

const optionNames = (property: NoteProperty) => (property.options ?? []).map(option => option.name).join(", ") || "no options yet";

/** A value the property takes, or the reason it does not. */
function checkValue(property: NoteProperty, value: unknown): void {
  const bad = (wanted: string) => new Error(`${property.name} is ${wanted}`);
  const option = (id: unknown) => {
    if (typeof id !== "string" || !(property.options ?? []).some(candidate => candidate.id === id)) throw new Error(`${property.name} has no option ${String(id)}; choose one of ${optionNames(property)}`);
  };
  switch (property.type) {
    case "text": case "url": if (typeof value !== "string") throw bad("text"); return;
    case "number": if (typeof value !== "number" || !Number.isFinite(value)) throw bad("a finite number"); return;
    case "checkbox": if (typeof value !== "boolean") throw bad("true or false"); return;
    case "select": case "status": option(value); return;
    case "multiSelect": case "relation": {
      if (!Array.isArray(value) || value.some(item => typeof item !== "string")) throw bad("a list");
      if (new Set(value).size !== value.length) throw new Error(`${property.name} lists the same item twice`);
      if (property.type === "multiSelect") value.forEach(option);
      return;
    }
    case "date": {
      const date = value as { start?: unknown; end?: unknown } | null;
      if (!date || typeof date !== "object" || typeof date.start !== "string" || Number.isNaN(Date.parse(date.start))) throw bad("{start, end?}");
      if (date.end !== undefined && (typeof date.end !== "string" || Date.parse(date.end) < Date.parse(date.start))) throw new Error(`${property.name} ends before it starts`);
      return;
    }
  }
}

/** A row's cells after a patch, checked whole: a refused patch writes nothing. */
function checkCells(page: MockPage, patch: Record<string, unknown>, parentId: string | null): Record<string, unknown> {
  const parent = parentId ? pages.find(candidate => candidate.note.id === parentId && !candidate.deleted && candidate.database) : undefined;
  if (!parent) throw new Error("only a row of a database has values");
  const properties = parent.database!.properties;
  const next = { ...(page.values ?? {}) };
  for (const [prop, value] of Object.entries(patch)) {
    const property = properties[prop];
    if (!property) throw new Error(`no property with id ${prop}; this database has ${Object.values(properties).map(candidate => candidate.name).sort().join(", ")}`);
    if (value === null) {
      delete next[prop];
    } else {
      checkValue(property, value);
      next[prop] = clone(value);
    }
  }
  return next;
}

function viewOf(input: NoteViewInput, database: MockDatabase): NoteDatabaseView {
  if (!["table", "board", "calendar", "list"].includes(input.type)) throw new Error(`a view is table, board, calendar or list, not ${input.type}`);
  const name = input.name.trim();
  if (!name) throw new Error("a view needs a name");
  const existing = input.id ? database.views[input.id] : undefined;
  const order = input.order ?? existing?.order ?? Math.max(-1, ...Object.values(database.views).map(view => view.order)) + 1;
  const view: NoteDatabaseView = { id: input.id ?? nextId("view"), name, type: input.type, order };
  if (input.filter != null) view.filter = clone(input.filter);
  if (input.sorts?.length) view.sorts = clone(input.sorts);
  if (input.groupBy) view.groupBy = clone(input.groupBy);
  if (input.dateProperty) view.dateProperty = input.dateProperty;
  if (input.properties?.length) view.properties = clone(input.properties);
  return view;
}

/** A view input whose property and option references may be names, with each
 *  name of a property in `properties` (or one of its options) made its id. */
function resolveViewNames(view: NoteViewInput, properties: Record<string, NoteProperty>): NoteViewInput {
  const property = (reference: string) => properties[reference] ? reference : Object.values(properties).find(candidate => fold(candidate.name) === fold(reference))?.id ?? reference;
  const filter = (tree: unknown): unknown => {
    const node = tree as Record<string, unknown>;
    for (const key of ["and", "or"]) if (Array.isArray(node?.[key])) return { [key]: (node[key] as unknown[]).map(filter) };
    if (typeof node?.property !== "string") return tree;
    const id = property(node.property);
    const condition: Record<string, unknown> = { ...node, property: id };
    const def = properties[id];
    if (def && OPTION_TYPES.includes(def.type) && "value" in node && node.op !== "inGroup") {
      const option = (raw: unknown) => typeof raw === "string" && !(def.options ?? []).some(candidate => candidate.id === raw) ? (def.options ?? []).find(candidate => fold(candidate.name) === fold(raw))?.id ?? raw : raw;
      condition.value = Array.isArray(node.value) ? node.value.map(option) : option(node.value);
    }
    return condition;
  };
  return {
    ...view,
    ...(view.filter != null ? { filter: filter(view.filter) } : {}),
    ...(view.sorts ? { sorts: view.sorts.map(sort => ({ ...sort, property: property(sort.property) })) } : {}),
    ...(view.groupBy ? { groupBy: { ...view.groupBy, property: property(view.groupBy.property) } } : {}),
    ...(view.dateProperty ? { dateProperty: property(view.dateProperty) } : {}),
    ...(view.properties ? { properties: view.properties.map(column => ({ ...column, id: property(column.id) })) } : {}),
  };
}

export function mockCreateDatabase(params: CreateDatabaseParams): NotePage {
  if (params.parentId) {
    const parent = live(params.parentId);
    if (parent.database) throw new Error("a database cannot be a row of another database");
  }
  const properties: Record<string, NoteProperty> = {};
  const inputs = params.properties?.length ? params.properties : [{ name: "Status", type: "status", options: [{ name: "To do", group: "todo" }, { name: "In progress", group: "inProgress" }, { name: "Done", group: "done" }] }];
  inputs.forEach((input, order) => {
    if (!PROPERTY_TYPES.includes(input.type)) throw new Error(`type must be one of ${PROPERTY_TYPES.join(", ")}`);
    const name = checkName(properties, input.name);
    const id = nextId("prop");
    properties[id] = { id, name, type: input.type, order, ...(OPTION_TYPES.includes(input.type) ? { options: newOptions(input.type, input.options) } : {}), ...(input.relation ? { relation: { databaseId: input.relation } } : {}), ...(input.dateFormat ? { dateFormat: input.dateFormat } : {}) };
  });
  const database: MockDatabase = { scope: "children", properties, views: {} };
  const views = params.views?.length ? params.views : [{ name: "Table", type: "table" }];
  views.forEach((input, order) => {
    // The caller cannot know the ids made here, so a first view may name its
    // properties and options, as the core resolves them.
    const view = viewOf({ ...resolveViewNames(input, properties), order: input.order ?? order }, database);
    // Checked against the draft before anything is made, as the core checks
    // first views inside its creating write.
    checkView({ database } as MockPage, view);
    database.views[view.id] = view;
  });
  const page = make("", PERSON, { title: params.title?.trim() || null, kind: "database", state: "open", parentId: params.parentId ?? null, workspaceId: params.workspaceId ?? null });
  page.database = database;
  return mockReadPage(page.note.id);
}

export function mockUpdateSchema(params: UpdateSchemaParams): NoteDatabase {
  const page = liveDatabase(params.id);
  const properties = clone(page.database!.properties);
  const cleared: { prop: string; options?: string[]; retype?: { from: NoteProperty; to: NoteProperty } }[] = [];
  const touched = new Set<string>();
  for (const change of params.changes) {
    if (change.op !== "add" && change.op !== "reorder") {
      const structural = change.op !== "setOptions";
      if (touched.has(change.propertyId) || (structural && touched.has(`${change.propertyId}:option`))) throw new Error("this batch changes the same property twice");
      touched.add(structural ? change.propertyId : `${change.propertyId}:option`);
    }
    const need = (id: string) => {
      const property = properties[id];
      if (!property) throw new Error(`no property with id ${id}`);
      return property;
    };
    switch (change.op) {
      case "add": {
        if (!PROPERTY_TYPES.includes(change.property.type)) throw new Error(`type must be one of ${PROPERTY_TYPES.join(", ")}`);
        const id = nextId("prop");
        const order = Math.max(-1, ...Object.values(properties).map(property => property.order)) + 1;
        properties[id] = { id, name: checkName(properties, change.property.name), type: change.property.type, order, ...(OPTION_TYPES.includes(change.property.type) ? { options: newOptions(change.property.type, change.property.options) } : {}), ...(change.property.relation ? { relation: { databaseId: change.property.relation } } : {}), ...(change.property.dateFormat ? { dateFormat: change.property.dateFormat } : {}) };
        break;
      }
      case "rename": need(change.propertyId).name = checkName(properties, change.name, change.propertyId); break;
      case "delete": need(change.propertyId); delete properties[change.propertyId]; break;
      case "retype": {
        const from = clone(need(change.propertyId));
        if (!PROPERTY_TYPES.includes(change.type)) throw new Error(`type must be one of ${PROPERTY_TYPES.join(", ")}`);
        if (change.type === "relation") throw new Error("a relation needs the database it points at; add one instead of retyping");
        const to: NoteProperty = { ...from, type: change.type, options: OPTION_TYPES.includes(change.type) ? (from.options ?? []).map(option => ({ ...option, group: change.type === "status" ? option.group ?? "todo" : undefined })) : undefined };
        delete to.relation;
        properties[change.propertyId] = to;
        cleared.push({ prop: change.propertyId, retype: { from, to } });
        break;
      }
      case "setOptions": {
        const property = need(change.propertyId);
        if (!OPTION_TYPES.includes(property.type)) throw new Error(`${property.name} does not have options`);
        const built: NoteOption[] = [];
        for (const input of change.options) {
          const existing = input.id ? (property.options ?? []).find(option => option.id === input.id) : undefined;
          if (input.id && !existing) throw new Error(`${property.name} has no option ${input.id}`);
          const name = input.name.trim();
          if (!name || built.some(option => fold(option.name) === fold(name))) throw new Error(`${property.name} already has an option named ${name}`);
          const group = property.type === "status" ? input.group ?? existing?.group ?? "todo" : undefined;
          const color = input.color ?? existing?.color;
          built.push({ id: existing?.id ?? nextId("option"), name, ...(color ? { color } : {}), ...(group ? { group } : {}) });
        }
        const gone = (property.options ?? []).filter(option => !built.some(kept => kept.id === option.id)).map(option => option.id);
        property.options = built;
        if (gone.length) cleared.push({ prop: change.propertyId, options: gone });
        break;
      }
      case "reorder": {
        const ids = Object.keys(properties);
        if (new Set(change.propertyIds).size !== change.propertyIds.length || change.propertyIds.length !== ids.length || change.propertyIds.some(id => !properties[id])) throw new Error(`reorder names every property once: ${ids.join(", ")}`);
        change.propertyIds.forEach((id, order) => { properties[id].order = order; });
        break;
      }
    }
  }
  page.database!.properties = properties;
  // Every saved view follows the new schema in this same write, as the core's.
  for (const [id, view] of Object.entries(page.database!.views)) page.database!.views[id] = pruneView(page, view);
  commit(page, page.history[page.history.length - 1], PERSON);
  // Values follow the schema: a dropped option leaves its rows, and a retype
  // converts what it can and clears the rest.
  for (const { prop, options, retype } of cleared) {
    for (const row of pages.filter(candidate => !candidate.deleted && candidate.note.parentId === page.note.id && candidate.values && prop in candidate.values)) {
      const value = row.values![prop];
      let next: unknown = value;
      if (options) next = Array.isArray(value) ? value.filter(item => !options.includes(item as string)) : options.includes(value as string) ? undefined : value;
      if (retype) next = retype.from.type === retype.to.type ? repairValue(retype.to, value) : convertValue(retype.from, retype.to, value);
      if (Array.isArray(next) && !next.length) next = undefined;
      if (next === undefined) delete row.values![prop];
      else row.values![prop] = next;
      commit(row, row.history[row.history.length - 1], PERSON);
    }
  }
  return databaseOf(page);
}

/** A view with every part the database's schema cannot run taken out, as the
 *  core's `prune_view`: a condition naming a removed property or option, or
 *  an op the property's type no longer takes, goes; a list keeps the items
 *  that still run; a group that empties goes. */
function pruneView(page: MockPage, view: NoteDatabaseView): NoteDatabaseView {
  const properties = page.database!.properties;
  const plan = { database: page, today: new Date().toISOString().slice(0, 10) };
  const runs = (filter: unknown, sorts: NoteSort[] = [], groupBy: NoteDatabaseView["groupBy"] = null) => {
    try {
      checkQuery(plan, filter ?? { and: [] }, sorts, groupBy ?? null);
      return true;
    } catch {
      return false;
    }
  };
  const prune = (filter: unknown): unknown => {
    if (!filter || typeof filter !== "object" || Array.isArray(filter)) return undefined;
    const node = filter as Record<string, unknown>;
    for (const key of ["and", "or"]) {
      const items = node[key];
      if (Array.isArray(items)) {
        if (!items.length) return filter;
        const kept = items.map(prune).filter(item => item !== undefined);
        return kept.length ? { [key]: kept } : undefined;
      }
    }
    if (runs(filter)) return filter;
    if (!Array.isArray(node.value)) return undefined;
    const kept = node.value.filter(item => runs({ ...node, value: [item] }));
    const narrowed = { ...node, value: kept };
    return kept.length && runs(narrowed) ? narrowed : undefined;
  };
  const known = (id: string) => id in properties || SYSTEM_PROPERTIES.includes(id);
  const filter = view.filter == null ? undefined : prune(view.filter);
  const groupBy = view.groupBy && runs(undefined, [], view.groupBy) ? view.groupBy : undefined;
  const dateProperty = view.dateProperty && (properties[view.dateProperty]?.type === "date" || ["created", "updated"].includes(view.dateProperty)) ? view.dateProperty : undefined;
  const { filter: _filter, groupBy: _groupBy, dateProperty: _dateProperty, ...rest } = view;
  return {
    ...rest,
    ...(filter === undefined ? {} : { filter }),
    sorts: (view.sorts ?? []).filter(sort => runs(undefined, [sort])),
    ...(groupBy ? { groupBy } : {}),
    ...(dateProperty ? { dateProperty } : {}),
    properties: (view.properties ?? []).filter(column => known(column.id)),
  };
}

/** A same-type retype, as the core's: a valid cell stays, and one a part-run
 *  retype left behind is converted from the type its shape says it came
 *  from, or cleared. */
function repairValue(to: NoteProperty, value: unknown): unknown {
  try {
    checkValue(to, value);
    return value;
  } catch {
    const shape = typeof value === "number" ? "number" : typeof value === "boolean" ? "checkbox" : typeof value === "string" ? "text" : Array.isArray(value) ? "multiSelect" : value && typeof value === "object" && "start" in value ? "date" : null;
    if (!shape) return undefined;
    // Converted into a scratch copy and checked against the real property, as
    // the core does: a repair never adds options to the schema.
    const converted = convertValue({ ...to, type: shape }, { ...to, options: [...(to.options ?? [])] }, value);
    try {
      checkValue(to, converted);
      return converted;
    } catch {
      return undefined;
    }
  }
}

/** What a retype makes of one value, as the core's `convert`. */
function convertValue(from: NoteProperty, to: NoteProperty, value: unknown): unknown {
  const optionName = (id: unknown) => (from.options ?? []).find(option => option.id === id)?.name;
  const optionId = (name: string) => {
    const found = (to.options ?? []).find(option => fold(option.name) === fold(name));
    if (found) return found.id;
    const created: NoteOption = { id: nextId("option"), name: name.trim(), ...(to.type === "status" ? { group: "todo" } : {}) };
    to.options = [...(to.options ?? []), created];
    return created.id;
  };
  const pair = `${from.type}>${to.type}`;
  if (pair === "number>text") return typeof value === "number" ? String(value) : undefined;
  if (pair === "text>number") { const number = Number(String(value).trim()); return String(value).trim() && Number.isFinite(number) ? number : undefined; }
  if (pair === "select>text" || pair === "status>text") return optionName(value);
  if (pair === "text>select" || pair === "text>status") return typeof value === "string" && value.trim() ? optionId(value) : undefined;
  if (pair === "select>multiSelect") return typeof value === "string" ? [value] : undefined;
  if (pair === "checkbox>text") return typeof value === "boolean" ? String(value) : undefined;
  if (pair === "text>checkbox") return { true: true, false: false }[fold(String(value)) as "true" | "false"];
  return undefined;
}

/** A view a database can hold, as the core's `check_view_def`: its columns,
 *  sorts and grouping name real properties, it compiles, and a calendar's
 *  date property is a date. */
function checkView(page: MockPage, view: NoteDatabaseView) {
  const properties = page.database!.properties;
  const known = (id: string) => id in properties || SYSTEM_PROPERTIES.includes(id);
  for (const id of [...(view.sorts ?? []).map(sort => sort.property), ...(view.groupBy ? [view.groupBy.property] : []), ...(view.dateProperty ? [view.dateProperty] : []), ...(view.properties ?? []).map(column => column.id)]) {
    if (!known(id)) throw new Error(`no property with id ${id}`);
  }
  checkQuery({ database: page, today: new Date().toISOString().slice(0, 10) }, view.filter, view.sorts ?? [], view.groupBy ?? null);
  if (view.dateProperty && !(properties[view.dateProperty]?.type === "date" || ["created", "updated"].includes(view.dateProperty))) throw new Error(`${view.dateProperty} is not a date property`);
}

export function mockSaveView(params: SaveViewParams): NoteDatabaseView {
  const page = liveDatabase(params.databaseId);
  if (params.view.id && !page.database!.views[params.view.id]) throw new Error(`no view ${params.view.id} to replace`);
  const view = viewOf(params.view, page.database!);
  // A saved view always compiles, as the core checks it.
  checkView(page, view);
  page.database!.views[view.id] = view;
  commit(page, page.history[page.history.length - 1], PERSON);
  return clone(view);
}

export function mockDeleteView(params: DeleteViewParams): void {
  const page = liveDatabase(params.databaseId);
  const views = page.database!.views;
  if (!views[params.viewId]) throw new Error("no such view");
  if (Object.keys(views).length === 1) throw new Error("a database keeps at least one view");
  delete views[params.viewId];
  commit(page, page.history[page.history.length - 1], PERSON);
}

// The planner's rules, run over the mock pages.

type Cell = string | number | null;

interface Plan {
  database: MockPage;
  today: string;
}

const dayOf = (raw: string): string | null => {
  const time = Date.parse(raw.length === 10 ? `${raw}T00:00:00Z` : raw);
  return Number.isNaN(time) ? null : new Date(time).toISOString().slice(0, 10);
};
const addDays = (day: string, days: number) => new Date(Date.parse(`${day}T00:00:00Z`) + days * 86_400_000).toISOString().slice(0, 10);

function withinRange(value: string, today: string): [string, string] {
  const weekday = (new Date(`${today}T00:00:00Z`).getUTCDay() + 6) % 7;
  switch (value) {
    case "pastWeek": return [addDays(today, -7), today];
    case "nextWeek": return [today, addDays(today, 7)];
    case "thisWeek": return [addDays(today, -weekday), addDays(today, 6 - weekday)];
    case "thisMonth": {
      const [year, month] = today.split("-").map(Number);
      const last = new Date(Date.UTC(year, month, 0)).toISOString().slice(0, 10);
      return [`${today.slice(0, 7)}-01`, last];
    }
    case "pastMonth": return [addDays(today, -30), today];
    case "nextMonth": return [today, addDays(today, 30)];
  }
  throw new Error("within takes one of pastWeek, nextWeek, thisWeek, thisMonth, pastMonth, nextMonth");
}

const NEGATIONS: Record<string, string> = { notContains: "contains", notEquals: "equals", ne: "eq", isNot: "is", isNoneOf: "isAnyOf", isEmpty: "isNotEmpty" };

function family(property: NoteProperty | string): string {
  if (typeof property === "string") {
    return ({ title: "text", createdBy: "text", kind: "choice", state: "choice", project: "choice", tags: "list", chats: "chats", parent: "parent", created: "date", updated: "date" } as Record<string, string>)[property];
  }
  return ({ text: "text", url: "text", number: "number", date: "date", select: "choice", status: "choice", multiSelect: "list", relation: "list", checkbox: "checkbox" } as Record<string, string>)[property.type];
}

const OPS: Record<string, string[]> = {
  text: ["contains", "notContains", "equals", "notEquals", "startsWith", "isEmpty", "isNotEmpty"],
  number: ["eq", "ne", "lt", "lte", "gt", "gte", "isEmpty", "isNotEmpty"],
  date: ["before", "after", "onOrBefore", "onOrAfter", "equals", "isEmpty", "isNotEmpty", "within"],
  choice: ["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty"],
  list: ["contains", "notContains", "containsAny", "isEmpty", "isNotEmpty"],
  checkbox: ["is"],
  chats: ["isEmpty", "isNotEmpty"],
  parent: ["isEmpty", "isNotEmpty", "equals", "notEquals"],
};

function resolveProperty(plan: Plan, id: string): NoteProperty | string {
  const property = plan.database.database!.properties[id];
  if (property) return property;
  if (SYSTEM_PROPERTIES.includes(id)) return id;
  const names = [...byOrder(Object.values(plan.database.database!.properties)).map(candidate => `${candidate.name} (${candidate.id})`), ...SYSTEM_PROPERTIES];
  throw new Error(`no property ${id}; this database has ${names.join(", ")}`);
}

/** Check a filter, sorts and grouping without a row, as the core does when it
 *  compiles a query: an empty database refuses a bad view too. */
function checkQuery(plan: Plan, filter: unknown, sorts: NoteSort[], groupBy: { property: string; dateBucket?: string | null } | null) {
  const count = { n: 0 };
  const walk = (tree: unknown, depth: number) => {
    const node = tree as Record<string, unknown>;
    if (!node || typeof node !== "object" || Array.isArray(node)) throw new Error("a filter is {and: [...]}, {or: [...]} or {property, op, value}");
    for (const key of ["and", "or"]) {
      if (key in node) {
        if (Object.keys(node).length !== 1) throw new Error(`a ${key} group holds only its list`);
        if (depth >= 2) throw new Error("filter groups nest at most two levels");
        if (!Array.isArray(node[key])) throw new Error(`${key} takes a list`);
        (node[key] as unknown[]).forEach(item => walk(item, depth + 1));
        return;
      }
    }
    if (++count.n > 20) throw new Error("a filter holds at most 20 conditions");
    const extra = Object.keys(node).find(key => !["property", "op", "value"].includes(key));
    if (extra) throw new Error(`a condition has property, op and value, not ${extra}`);
    const property = resolveProperty(plan, node.property as string);
    const name = typeof property === "string" ? property : property.name;
    const op = node.op as string;
    const ops = typeof property !== "string" && property.type === "status" ? [...OPS.choice, "inGroup"] : OPS[family(property)];
    if (!ops.includes(op)) throw new Error(`${name} takes ${ops.join(", ")}, not ${String(op)}`);
    checkConditionValue(property, name, op, node.value, plan.today);
  };
  if (filter != null) walk(filter, 0);
  for (const sort of sorts) {
    const property = resolveProperty(plan, sort.property);
    if (!["asc", "desc"].includes(sort.direction)) throw new Error(`a sort direction is asc or desc, not ${sort.direction}`);
    if (typeof property === "string" ? ["tags", "chats", "parent"].includes(property) : ["multiSelect", "relation"].includes(property.type)) throw new Error(`${typeof property === "string" ? property : property.name} cannot be sorted`);
  }
  if (groupBy) {
    if (groupBy.dateBucket != null && !["day", "week", "month"].includes(groupBy.dateBucket)) throw new Error("dateBucket is day, week or month");
    const property = resolveProperty(plan, groupBy.property);
    const groupable = typeof property === "string" ? ["kind", "state", "project", "created", "updated"].includes(property) : ["select", "status", "multiSelect", "checkbox", "date"].includes(property.type);
    if (!groupable) throw new Error(`${typeof property === "string" ? property : property.name} cannot be grouped`);
  }
}

/** A condition's value has the shape its op takes, and names options that
 *  exist, as the core refuses otherwise. */
function checkConditionValue(property: NoteProperty | string, name: string, op: string, value: unknown, today: string) {
  const kind = family(property);
  const positive = NEGATIONS[op] ?? op;
  if (positive === "isNotEmpty") return;
  const known = typeof property === "string"
    ? ({ kind: KINDS, state: STATES } as Record<string, string[]>)[property]
    : OPTION_TYPES.includes(property.type) ? (property.options ?? []).map(option => option.id) : undefined;
  const choices = typeof property === "string" ? known?.join(", ") : (property.options ?? []).map(option => `${option.name} (${option.id})`).join(", ");
  const option = (id: unknown) => {
    if (typeof id !== "string") throw new Error(`${op} on ${name} takes text`);
    if (known && !known.includes(id)) throw new Error(`${name} has no option ${id}; choose one of ${choices}`);
  };
  const list = (): unknown[] => {
    if (!Array.isArray(value)) throw new Error(`${op} on ${name} takes a list`);
    return value;
  };
  switch (kind) {
    case "text": case "parent":
      if (typeof value !== "string") throw new Error(`${op} on ${name} takes text`);
      return;
    case "number":
      if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`${op} on ${name} takes a number`);
      return;
    case "date":
      if (positive === "within") { withinRange(String(value), today); return; }
      if (typeof value !== "string" || dayOf(value) === null) throw new Error(`${op} on ${name} takes YYYY-MM-DD or an RFC 3339 time`);
      return;
    case "checkbox":
      if (typeof value !== "boolean") throw new Error(`is on ${name} takes true or false`);
      return;
    case "choice":
      if (positive === "inGroup") {
        if (!STATUS_GROUPS.includes(value as string)) throw new Error(`inGroup takes one of ${STATUS_GROUPS.join(", ")}`);
        return;
      }
      if (positive === "isAnyOf") list().forEach(option);
      else option(value);
      return;
    case "list": {
      const relation = typeof property !== "string" && property.type === "relation";
      // Options must exist; a relation's pages and a tag are free text.
      const each = relation || property === "tags"
        ? (item: unknown) => { if (typeof item !== "string") throw new Error(`${op} on ${name} takes text`); }
        : option;
      if (positive === "containsAny") list().forEach(each);
      else each(value);
      return;
    }
  }
}

/** Whether one row matches a filter tree. */
function matches(plan: Plan, row: MockPage, tree: unknown, depth = 0, count = { n: 0 }): boolean {
  const node = tree as Record<string, unknown>;
  for (const key of ["and", "or"] as const) {
    if (key in node) {
      if (depth >= 2) throw new Error("filter groups nest at most two levels");
      const items = node[key] as unknown[];
      if (!items.length) return true;
      const results = items.map(item => matches(plan, row, item, depth + 1, count));
      return key === "and" ? results.every(Boolean) : results.some(Boolean);
    }
  }
  if (++count.n > 20) throw new Error("a filter holds at most 20 conditions");
  const id = node.property as string;
  const op = node.op as string;
  const value = node.value;
  const property = resolveProperty(plan, id);
  const kind = family(property);
  const ops = typeof property !== "string" && property.type === "status" ? [...OPS.choice, "inGroup"] : OPS[kind];
  if (!ops.includes(op)) throw new Error(`${typeof property === "string" ? property : property.name} takes ${ops.join(", ")}, not ${op}`);
  const positive = NEGATIONS[op] ?? op;
  const negate = op in NEGATIONS;
  const note = row.note;
  const cell = typeof property === "string" ? undefined : row.values?.[property.id];
  const text = (raw: string | null | undefined): boolean => {
    const present = raw != null && raw !== "";
    if (positive === "isNotEmpty") return present;
    if (!present) return false;
    const needle = asciiLower(String(value ?? ""));
    const folded = asciiLower(raw!);
    return positive === "contains" ? folded.includes(needle) : positive === "equals" ? folded === needle : folded.startsWith(needle);
  };
  const date = (day: string | null): boolean => {
    if (positive === "isNotEmpty") return day !== null;
    if (day === null) return false;
    if (positive === "within") { const [from, to] = withinRange(String(value), plan.today); return day >= from && day <= to; }
    const wanted = dayOf(String(value));
    if (wanted === null) throw new Error(`${op} takes YYYY-MM-DD or an RFC 3339 time`);
    return { before: day < wanted, after: day > wanted, onOrBefore: day <= wanted, onOrAfter: day >= wanted, equals: day === wanted }[positive as "before"]!;
  };
  const choice = (raw: string | null | undefined, groupOf?: (id: string) => string | undefined): boolean => {
    const present = raw != null && raw !== "";
    if (positive === "isNotEmpty") return present;
    if (!present) return false;
    if (positive === "is") return raw === value;
    if (positive === "isAnyOf") return (value as string[]).includes(raw!);
    return groupOf?.(raw!) === value;
  };
  const list = (items: string[]): boolean => {
    if (positive === "isNotEmpty") return items.length > 0;
    if (positive === "contains") return items.includes(value as string);
    return (value as string[]).some(wanted => items.includes(wanted));
  };
  let result: boolean;
  if (typeof property === "string") {
    switch (property) {
      case "title": result = text(note.title); break;
      case "createdBy": result = text(note.createdBy); break;
      case "kind": result = choice(note.kind); break;
      case "state": result = choice(note.state); break;
      case "project": result = choice(note.workspaceId); break;
      case "tags": {
        // Tag values read as stored tags are kept: trimmed, without `#`, folded.
        const tag = (raw: unknown) => String(raw).trim().replace(/^#+/, "").toLowerCase();
        const wanted = Array.isArray(value) ? value.map(tag) : tag(value);
        result = positive === "isNotEmpty" ? note.tags.length > 0 : positive === "contains" ? note.tags.includes(wanted as string) : (wanted as string[]).some(item => note.tags.includes(item));
        break;
      }
      case "chats": result = note.sessionIds.length > 0; break;
      case "parent": result = positive === "isNotEmpty" ? note.parentId != null : note.parentId === value; break;
      default: result = date(dayOf(property === "created" ? note.createdAt : note.updatedAt));
    }
  } else {
    switch (property.type) {
      case "text": case "url": result = text(typeof cell === "string" ? cell : null); break;
      case "number": {
        if (typeof cell !== "number") { result = false; break; }
        const wanted = value as number;
        result = positive === "isNotEmpty" || ({ eq: cell === wanted, lt: cell < wanted, lte: cell <= wanted, gt: cell > wanted, gte: cell >= wanted }[positive as "eq"] ?? false);
        break;
      }
      case "date": { const start = (cell as { start?: string } | undefined)?.start; result = date(start ? dayOf(start) : null); break; }
      case "select": case "status": result = choice(cell as string | undefined, option => property.options?.find(candidate => candidate.id === option)?.group ?? undefined); break;
      case "multiSelect": case "relation": result = list(Array.isArray(cell) ? cell as string[] : []); break;
      default: result = cell === true ? value === true : value === false;
    }
  }
  return result !== negate;
}

/** A sort key as the core's SQL computes it; null is an empty value. */
function sortKey(plan: Plan, row: MockPage, id: string): Cell {
  const property = resolveProperty(plan, id);
  const note = row.note;
  if (typeof property === "string") {
    switch (property) {
      case "title": return note.title ? asciiLower(note.title) : null;
      case "createdBy": return note.createdBy ? asciiLower(note.createdBy) : null;
      case "kind": return KINDS.includes(note.kind) ? KINDS.indexOf(note.kind) : null;
      case "state": return STATES.includes(note.state) ? STATES.indexOf(note.state) : null;
      case "project": return note.workspaceId ?? null;
      case "created": return note.createdAt;
      case "updated": return note.updatedAt;
    }
    throw new Error(`${property} cannot be sorted`);
  }
  const cell = row.values?.[property.id];
  switch (property.type) {
    case "text": case "url": return typeof cell === "string" && cell ? asciiLower(cell) : null;
    case "number": return typeof cell === "number" ? cell : null;
    case "date": return dateKey(property, cell);
    case "checkbox": return cell === true ? "true" : "false";
    case "select": case "status": { const index = (property.options ?? []).findIndex(option => option.id === cell); return index < 0 ? null : index; }
  }
  throw new Error(`${property.name} holds a list and cannot be sorted`);
}

/** The core's `date_v`: a datetime's UTC instant, a date's UTC day. */
function dateKey(property: NoteProperty, cell: unknown): string | null {
  const start = (cell as { start?: string } | undefined)?.start;
  if (!start) return null;
  const time = Date.parse(start.length === 10 ? `${start}T00:00:00Z` : start);
  if (Number.isNaN(time)) return null;
  const iso = new Date(time).toISOString();
  return property.dateFormat === "datetime" ? iso : iso.slice(0, 10);
}

/** Text in code point order, which is the core's UTF-8 byte order; plain
 *  `<` compares UTF-16 units and puts an emoji before U+E000 to U+FFFF. */
function compareText(x: string, y: string): number {
  const [a, b] = [Array.from(x), Array.from(y)];
  for (let index = 0; index < Math.min(a.length, b.length); index++) {
    const difference = a[index].codePointAt(0)! - b[index].codePointAt(0)!;
    if (difference) return difference < 0 ? -1 : 1;
  }
  return a.length - b.length < 0 ? -1 : a.length > b.length ? 1 : 0;
}

function compareKeys(left: Cell[], right: Cell[], descending: boolean[], a: Note, b: Note): number {
  for (let index = 0; index < left.length; index++) {
    const [x, y] = [left[index], right[index]];
    if (x === null && y === null) continue;
    if (x === null) return 1;
    if (y === null) return -1;
    const order = typeof x === "string" && typeof y === "string" ? compareText(x, y) : x < y ? -1 : x > y ? 1 : 0;
    if (order) return descending[index] ? -order : order;
  }
  if (a.updatedAt !== b.updatedAt) return a.updatedAt < b.updatedAt ? 1 : -1;
  return a.id < b.id ? 1 : a.id > b.id ? -1 : 0;
}

/** URL-safe base64 of UTF-8, as the core writes a cursor: a title key in any
 *  script survives, where `btoa` alone takes only Latin-1. */
function base64Url(text: string): string {
  let binary = "";
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function fromBase64Url(text: string): string {
  const binary = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
  return new TextDecoder().decode(Uint8Array.from(binary, character => character.charCodeAt(0)));
}
const encodeCursor = (keys: Cell[], note: Note) => base64Url(JSON.stringify({ k: keys, u: note.updatedAt, i: note.id }));
function decodeCursor(cursor: string, sorts: number): { k: Cell[]; u: string; i: string } {
  try {
    const body = JSON.parse(fromBase64Url(cursor));
    if (Array.isArray(body.k) && body.k.length === sorts && typeof body.u === "string" && typeof body.i === "string") return body;
  } catch { /* fall through */ }
  throw new Error("this cursor does not belong to this query");
}

function groupKeys(plan: Plan, row: MockPage, group: { property: string; dateBucket?: string | null }): string[] {
  const property = resolveProperty(plan, group.property);
  const bucket = (raw: string | null) => {
    if (!raw) return [];
    const day = raw.slice(0, 10);
    if (group.dateBucket === "month") return [day.slice(0, 7)];
    if (group.dateBucket === "week") { const weekday = (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7; return [addDays(day, -weekday)]; }
    return [day];
  };
  if (typeof property === "string") {
    if (property === "kind") return [row.note.kind];
    if (property === "state") return [row.note.state];
    if (property === "project") return row.note.workspaceId ? [row.note.workspaceId] : [];
    if (property === "created") return bucket(row.note.createdAt);
    if (property === "updated") return bucket(row.note.updatedAt);
    throw new Error(`${property} cannot be grouped`);
  }
  const cell = row.values?.[property.id];
  switch (property.type) {
    case "select": case "status": return typeof cell === "string" ? [cell] : [];
    case "multiSelect": return Array.isArray(cell) ? cell as string[] : [];
    case "checkbox": return [cell === true ? "true" : "false"];
    case "date": return bucket(dateKey(property, cell));
  }
  throw new Error(`${property.name} cannot be grouped; group by a select, status, multi-select, checkbox or date`);
}

function buckets(plan: Plan, rows: MockPage[], group: { property: string; dateBucket?: string | null }, limit: number): DatabaseGroup[] {
  const property = resolveProperty(plan, group.property);
  const title = (word: string) => word.charAt(0).toUpperCase() + word.slice(1);
  let fixed: [string, string][] | null = null;
  let none = true;
  if (typeof property === "string") {
    if (property === "kind") fixed = KINDS.map(kind => [kind, title(kind)]);
    if (property === "state") fixed = STATES.map(state => [state, title(state)]);
  } else if (OPTION_TYPES.includes(property.type)) {
    fixed = (property.options ?? []).map(option => [option.id, option.name]);
  } else if (property.type === "checkbox") {
    fixed = [["true", "Checked"], ["false", "Unchecked"]];
    none = false;
  }
  const keyed = rows.map(row => ({ row, keys: groupKeys(plan, row, group) }));
  const keys: [string, string][] = fixed ?? [...new Set(keyed.flatMap(entry => entry.keys))].sort().map(key => [key, key]);
  const out: DatabaseGroup[] = keys.map(([key, label]) => ({ key, label, count: 0, rows: [] }));
  if (none) out.push({ key: "", label: "No value", count: 0, rows: [] });
  for (const { row, keys: rowKeys } of keyed) {
    const known = rowKeys.filter(key => key && out.some(entry => entry.key === key));
    for (const key of known.length ? known : [""]) {
      const entry = out.find(candidate => candidate.key === key);
      if (!entry) continue;
      entry.count += 1;
      if (entry.rows.length < limit) entry.rows.push(rowOf(row, plan.database));
    }
  }
  return fixed && none ? out : out.filter(entry => entry.key !== "" || entry.count > 0);
}

/** A row with the cells of the properties its database still has. */
function rowOf(page: MockPage, database: MockPage): DatabaseRow {
  const properties = database.database!.properties;
  return { note: clone(page.note), values: Object.fromEntries(Object.entries(page.values ?? {}).filter(([prop]) => prop in properties).map(([prop, value]) => [prop, clone(value)])) };
}

export function mockQueryDatabase(params: QueryDatabaseParams): DatabaseQueryResult {
  const database = liveDatabase(params.databaseId);
  const view = params.viewId ? database.database!.views[params.viewId] : undefined;
  if (params.viewId && !view) throw new Error(`no view ${params.viewId}; this database has ${Object.values(database.database!.views).map(candidate => `${candidate.name} (${candidate.id})`).join(", ")}`);
  const filter = params.filter ?? view?.filter;
  const sorts: NoteSort[] = params.sorts ?? view?.sorts ?? [];
  const groupBy = params.groupBy ?? view?.groupBy ?? null;
  if (sorts.length > 5) throw new Error("a query sorts by at most 5 properties");
  const today = params.today ?? new Date().toISOString().slice(0, 10);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(today) || dayOf(today) !== today) throw new Error(`today is YYYY-MM-DD, not ${today}`);
  const plan: Plan = { database, today };
  checkQuery(plan, filter, sorts, groupBy);
  const query = params.query?.trim();
  const candidates = pages.filter(page => !page.deleted && (database.database!.scope === "space" ? page.note.id !== database.note.id : page.note.parentId === database.note.id));
  const rows = candidates
    .filter(page => !query || searchHaystack(query, `${page.note.title ?? ""}\n${page.history[page.history.length - 1]}`))
    .filter(page => filter == null || matches(plan, page, filter));
  const descending = sorts.map(sort => sort.direction === "desc");
  const keyed = rows.map(page => ({ page, keys: sorts.map(sort => sortKey(plan, page, sort.property)) }));
  keyed.sort((a, b) => compareKeys(a.keys, b.keys, descending, a.page.note, b.page.note));
  const limit = Math.min(Math.max(params.limit ?? 100, 1), 500);
  let start = 0;
  if (params.cursor) {
    const cursor = decodeCursor(params.cursor, sorts.length);
    const anchor = { updatedAt: cursor.u, id: cursor.i } as Note;
    start = keyed.findIndex(entry => compareKeys(entry.keys, cursor.k, descending, entry.page.note, anchor) > 0);
    if (start < 0) start = keyed.length;
  }
  const page = keyed.slice(start, start + limit);
  const result: DatabaseQueryResult = { rows: page.map(entry => rowOf(entry.page, database)), total: keyed.length };
  if (start + page.length < keyed.length && page.length) result.nextCursor = encodeCursor(page[page.length - 1].keys, page[page.length - 1].page.note);
  if (groupBy) result.groups = buckets(plan, keyed.map(entry => entry.page), groupBy, Math.min(Math.max(params.groupLimit ?? 50, 1), 200));
  return result;
}

// Last, so every table and constant the seed reads is defined.
seed();
