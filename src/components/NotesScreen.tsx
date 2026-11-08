// Bridge Notes: capture in your own words, open any page (yours or one an
// agent wrote), structure it or turn it into tasks when you ask, and start a
// Bridge chat from any task with or without a project.
//
// The list and the open page re-read on every `notes/page_changed`, so a page
// an agent writes appears live; the editor keeps unsaved typing and lets the
// core merge it. The list collapses (`toggle-notes-list`) so the page can take the width.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ChevronRight, MoreHorizontal, PanelLeft, Plus, Search, Settings2, Table2 } from "lucide-react";
import { bridgeApi } from "../api";
import { taskBrief } from "../notesBrief";
import { noteDisplayTitle, pageTitled, plainPreview } from "../notesDraft";
import type { AdapterDescriptor, Note, NoteDatabase, NoteDatabaseView, NotePage, NotesList, NotesSettings, Session, Workspace } from "../types";
import { SEEDED_VIEWS, seededKind, type SeededKind } from "../notesSystemViews";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { chordLabel } from "../keymap";
import { relativeTime } from "../connectorSurface";
import { Kbd } from "./ui/kbd";
import { HarnessMark } from "./harnessMarks";
import { MenuDivider, Pill, Popover, PrimaryButton, StatusGlyph } from "./notes/NotesUi";
import { viewInputOf } from "./notes/database/formatValue";
import { useViewOverride } from "./notes/database/useViewOverride";
import { ViewToolbar } from "./notes/database/ViewToolbar";
import { chatName } from "./sidebarChats";
import { NotePageView } from "./notes/NotePageView";
import type { StartChoice } from "./notes/NotePickers";

export const JOB_POLL_MS = 1500;

/** The empty-state sentence for a seeded tab. */
const EMPTY: Partial<Record<SeededKind, string>> = {
  tasks: "No tasks yet. Open a page and choose Turn into tasks.",
  inbox: "The Inbox is empty.",
  archived: "Nothing archived. Set a page's status to Archived to put it here.",
  all: "Nothing here yet. Type above and press Enter.",
};

export interface NoteChatRequest { message: string; workspaceId: string | null; harness: string }

/** What the host lends the Notes chat panel: the open page, as it stands,
 *  and a way to close the panel. */
export interface NotesCopilotHost {
  render: (page: NotePage | null, onClose: () => void) => ReactNode;
}

const CHAT_OPEN_KEY = "bridge.notes.chatOpen";
const readChatOpen = () => { try { return localStorage.getItem(CHAT_OPEN_KEY) === "1"; } catch { return false; } };
const LIST_OPEN_KEY = "bridge.notes.listOpen";
const readListOpen = () => { try { return localStorage.getItem(LIST_OPEN_KEY) !== "0"; } catch { return true; } };
const EXPANDED_KEY = "bridge.notes.expandedPages";
const readExpanded = (): Set<string> => { try { return new Set<string>(JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? "[]") as string[]); } catch { return new Set(); } };

export interface NotesScreenProps {
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  sessions: Session[];
  /** The chat default harness, preselected in every picker. */
  defaultHarness: string | null;
  /** A new value focuses the capture box (`capture-note`). */
  captureRequest?: number;
  /** A new value collapses or expands the notes list (`toggle-notes-list`). */
  listToggleRequest?: number;
  /** Start a Bridge chat with `message` as its first turn; resolves to its id. */
  onStartChat: (request: NoteChatRequest) => Promise<string | undefined>;
  onOpenChat: (sessionId: string) => void;
  onError: (message: string) => void;
  copilot?: NotesCopilotHost;
  /** How often to re-read while a Structure or Tasks pass is queued. */
  pollMs?: number;
}

const message = (error: unknown) => error instanceof Error ? error.message : String(error);
/** The harness that wrote a page, when an agent did. */
const agentHarness = (note: Note) => /^agent:([^:]+):/.exec(note.lastActor)?.[1] ?? /^agent:([^:]+):/.exec(note.createdBy)?.[1];

export function NotesScreen({ adapters, workspaces, sessions, defaultHarness, captureRequest, listToggleRequest, onStartChat, onOpenChat, onError, copilot, pollMs = JOB_POLL_MS }: NotesScreenProps) {
  const [list, setList] = useState<NotesList | null>(null);
  // Every note, search or not: links, moves and breadcrumbs resolve against
  // all pages, never just the ones a search shows.
  const [everyNote, setEveryNote] = useState<Note[] | null>(null);
  // The tabs are the system database's saved views; each tab's rows come
  // from notes/query_database. An edited tab shows an unsaved override until
  // it is saved for everyone or reset.
  const [system, setSystem] = useState<{ id: string; database: NoteDatabase } | null>(null);
  const [activeViewId, setActiveViewId] = useState<string | null>(null);
  const [tabNotes, setTabNotes] = useState<Note[] | null>(null);
  const [editingView, setEditingView] = useState(false);
  const override = useViewOverride();
  const tabViews = useMemo(() => [...(system?.database.views ?? [])].sort((a, b) => a.order - b.order), [system]);
  const activeView: NoteDatabaseView | undefined = tabViews.find(view => view.id === activeViewId) ?? tabViews[0];
  const effective = activeView ? override.effective(activeView) : null;
  const viewDirty = activeView ? override.dirty(activeView) : false;
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [page, setPage] = useState<NotePage | null>(null);
  // Captures on their way to the core. A failed one stays here, with its
  // words, until it is retried or discarded: nothing typed is ever dropped.
  const [pending, setPending] = useState<{ key: string; body: string; failed?: string }[]>([]);
  const [startingId, setStartingId] = useState<string | null>(null);
  const [chatOpen, setChatOpen] = useState(readChatOpen);
  const [focusCapture, setFocusCapture] = useState(0);
  const toggleChat = useCallback(() => setChatOpen(current => {
    try { localStorage.setItem(CHAT_OPEN_KEY, current ? "0" : "1"); } catch { /* storage off */ }
    return !current;
  }), []);
  const [listOpen, setListOpen] = useState(readListOpen);
  const [expandedPages, setExpandedPages] = useState<Set<string>>(readExpanded);
  const changeExpanded = useCallback((change: (next: Set<string>) => void) => setExpandedPages(current => {
    const next = new Set(current);
    change(next);
    if (next.size === current.size && [...next].every(id => current.has(id))) return current;
    try { localStorage.setItem(EXPANDED_KEY, JSON.stringify([...next])); } catch { /* storage off */ }
    return next;
  }), []);
  const toggleExpand = useCallback((id: string) => changeExpanded(next => { if (next.has(id)) next.delete(id); else next.add(id); }), [changeExpanded]);
  const toggleList = useCallback(() => setListOpen(current => {
    try { localStorage.setItem(LIST_OPEN_KEY, current ? "0" : "1"); } catch { /* storage off */ }
    return !current;
  }), []);
  useEffect(() => { if (listToggleRequest) toggleList(); }, [listToggleRequest, toggleList]);
  const selected = useRef<string | null>(null);
  selected.current = selectedId;
  const queryRef = useRef("");
  queryRef.current = query;
  const fail = useCallback((error: unknown) => onError(message(error)), [onError]);

  // The ref moves with the click, not the next render: a read that resolves
  // first must still know which page it is for.
  const select = useCallback((id: string | null) => {
    selected.current = id;
    setSelectedId(id);
    if (!id) setPage(null);
  }, []);
  // List reads overlap too (a search typed while a refresh is out); only the
  // newest one may replace what is shown.
  const listRequest = useRef(0);
  // Set by any change, cleared once a whole list lands: a search keystroke
  // that cuts in on a change's read still reads the whole list again.
  const wholeStale = useRef(true);
  // `searchOnly`: only the query changed, so the whole list still holds.
  // The open tab's rows. Reads overlap; only the newest lands.
  const systemRef = useRef<{ id: string; database: NoteDatabase } | null>(null);
  const tabRef = useRef<{ view: NoteDatabaseView | undefined; spelled: unknown }>({ view: undefined, spelled: null });
  tabRef.current = { view: activeView, spelled: viewDirty && effective ? effective : null };
  const tabRequest = useRef(0);
  const rereadSystem = useCallback(async () => {
    const sys = systemRef.current;
    if (!sys) return;
    try {
      const read = await bridgeApi.readNotePage(sys.id);
      if (read.database) { systemRef.current = { id: sys.id, database: read.database }; setSystem(systemRef.current); }
    } catch (error) { fail(error); }
  }, [fail]);
  const refreshTab = useCallback(async () => {
    const sys = systemRef.current;
    const views = [...(sys?.database.views ?? [])].sort((a, b) => a.order - b.order);
    const view = tabRef.current.view && views.some(candidate => candidate.id === tabRef.current.view!.id) ? tabRef.current.view : views[0];
    if (!sys || !view) return;
    const ticket = ++tabRequest.current;
    const spelled = tabRef.current.view?.id === view.id ? tabRef.current.spelled as { filter: unknown; sorts: NoteDatabaseView["sorts"]; groupBy: NoteDatabaseView["groupBy"] } | null : null;
    const base = spelled ? { filter: spelled.filter, sorts: spelled.sorts, groupBy: spelled.groupBy } : { viewId: view.id };
    try {
      const notes: Note[] = [];
      let cursor: string | null = null;
      do {
        const result = await bridgeApi.queryDatabase({ databaseId: sys.id, ...base, query: queryRef.current.trim() || null, limit: 500, cursor });
        if (ticket !== tabRequest.current) return;
        notes.push(...result.rows.map(row => row.note));
        cursor = result.nextCursor ?? null;
      } while (cursor);
      setTabNotes(notes);
    } catch (error) {
      if (ticket === tabRequest.current) fail(error);
    }
  }, [fail]);
  const refreshList = useCallback(async (searchOnly = false) => {
    const ticket = ++listRequest.current;
    if (!searchOnly) wholeStale.current = true;
    try {
      // Archived pages come too, for the Archived tab; the others leave them out.
      const query = queryRef.current.trim() || null;
      const [next, whole] = await Promise.all([
        bridgeApi.listNotes({ query, includeArchived: true }),
        query && wholeStale.current ? bridgeApi.listNotes({ query: null, includeArchived: true }) : null,
      ]);
      if (ticket !== listRequest.current) return;
      setList(next);
      // The system database (the tabs) is read once here; after that only when
      // a change names it or a tab is edited from here.
      if (next.systemDatabaseId && systemRef.current?.id !== next.systemDatabaseId) {
        const read = await bridgeApi.readNotePage(next.systemDatabaseId);
        if (ticket !== listRequest.current) return;
        if (read.database) {
          systemRef.current = { id: next.systemDatabaseId, database: read.database };
          setSystem(systemRef.current);
        }
      }
      await refreshTab();
      if (whole || !query) {
        setEveryNote((whole ?? next).notes);
        wholeStale.current = false;
      }
    } catch (error) {
      if (ticket === listRequest.current) fail(error);
    }
  }, [fail, refreshTab]);
  // Reads of the same page can overlap (several live updates in a row). Only
  // the newest request applies, and never a version older than the one shown.
  const pageRequest = useRef(0);
  const showPage = useCallback((next: NotePage, ticket: number) => {
    if (ticket !== pageRequest.current || selected.current !== next.note.id) return;
    setPage(current => current?.note.id === next.note.id && current.seq > next.seq ? current : next);
  }, []);
  const refreshPage = useCallback(async () => {
    const id = selected.current;
    if (!id) return;
    const ticket = ++pageRequest.current;
    try {
      showPage(await bridgeApi.readNotePage(id), ticket);
    } catch {
      // Deleted elsewhere: close it.
      if (selected.current === id && ticket === pageRequest.current) select(null);
    }
  }, [select, showPage]);
  const refreshAll = useCallback(async () => { await Promise.all([refreshList(), refreshPage()]); }, [refreshList, refreshPage]);

  useEffect(() => {
    const timer = setTimeout(() => void refreshList(true), query ? 200 : 0);
    return () => clearTimeout(timer);
  }, [query, refreshList]);

  // Live: every committed change, from anyone, re-reads. Bursts (a pass adding
  // several tasks) coalesce into one read.
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    let burst: ReturnType<typeof setTimeout> | undefined;
    let tabsChanged = false;
    void bridgeApi.onNotesPageChanged(payload => {
      if (payload.pageId === systemRef.current?.id) tabsChanged = true;
      clearTimeout(burst);
      burst = setTimeout(() => {
        if (tabsChanged) void rereadSystem();
        tabsChanged = false;
        void refreshAll();
      }, 30);
    }).then(stop => { if (alive) unlisten = stop; else stop(); });
    return () => { alive = false; clearTimeout(burst); unlisten?.(); };
  }, [refreshAll, rereadSystem]);

  // A failed pass announces nothing, so a queued one is watched until it settles.
  const waiting = list?.jobs.some(job => job.status === "pending" || job.status === "running") ?? false;
  useEffect(() => {
    if (!waiting) return;
    const timer = setTimeout(() => void refreshAll(), pollMs);
    return () => clearTimeout(timer);
  }, [waiting, list, pollMs, refreshAll]);

  const open = useCallback(async (id: string) => {
    select(id);
    setPage(current => current?.note.id === id ? current : null);
    const ticket = ++pageRequest.current;
    try {
      showPage(await bridgeApi.readNotePage(id), ticket);
    } catch (error) { fail(error); }
  }, [fail, select, showPage]);

  const send = async (key: string, body: string) => {
    try {
      await bridgeApi.captureNote(body);
      setPending(current => current.filter(item => item.key !== key));
      await refreshList();
    } catch (error) {
      setPending(current => current.map(item => item.key === key ? { ...item, failed: message(error) } : item));
      fail(error);
    }
  };
  const capture = (body: string) => {
    const key = crypto.randomUUID();
    setPending(current => [{ key, body }, ...current]);
    void send(key, body);
  };
  const retry = (key: string, body: string) => {
    setPending(current => current.map(item => item.key === key ? { key, body } : item));
    void send(key, body);
  };

  const createDatabase = async () => {
    try {
      const made = await bridgeApi.createDatabase({ title: "Untitled database" });
      await refreshList();
      await open(made.note.id);
    } catch (error) { fail(error); }
  };
  const createPage = async () => {
    try {
      const note = await bridgeApi.createNotePage({ title: null, content: "" });
      await refreshList();
      await open(note.id);
    } catch (error) { fail(error); }
  };
  // The `/` menu's Page: made under its parent, left closed. A [[ menu makes
  // a page named so at the top level instead.
  // A titled page being made is shared until the list shows it, so a second
  // click on the same link cannot make it twice.
  const making = useRef(new Map<string, { made: Promise<{ id: string; title: string } | null>; id?: string }>());
  const createPageFor = useCallback((parentId: string | null, title: string | null) => {
    const key = title === null ? null : `${parentId ?? ""}\n${title.trim().toLowerCase()}`;
    const pending = key === null ? undefined : making.current.get(key);
    if (pending) return pending.made;
    const entry: { made: Promise<{ id: string; title: string } | null>; id?: string } = { made: null! };
    entry.made = (async () => {
      try {
        const note = await bridgeApi.createNotePage({ title, content: "", ...(parentId ? { parentId } : {}) });
        entry.id = note.id;
        await refreshList();
        return { id: note.id, title: noteDisplayTitle(note) };
      } catch (error) {
        if (key !== null) making.current.delete(key);
        fail(error);
        return null;
      }
    })();
    if (key !== null) making.current.set(key, entry);
    return entry.made;
  }, [fail, refreshList]);

  const notes = list?.notes ?? [];
  const allNotes = everyNote ?? notes;
  const byId = useMemo(() => new Map(allNotes.map(note => [note.id, note])), [allNotes]);
  // A made page leaves the in-flight set once the list holds it.
  useEffect(() => {
    for (const [key, entry] of making.current) if (entry.id && byId.has(entry.id)) making.current.delete(key);
  }, [byId]);
  // A [[ link opens the page titled so, making it first when none is there.
  // Like the core: by the title a list shows, case and spacing aside, oldest
  // first.
  const openByTitle = useCallback((title: string) => {
    const found = pageTitled(allNotes, title);
    if (found) { void open(found.id); return; }
    void createPageFor(null, title).then(page => { if (page) void open(page.id); });
  }, [allNotes, open, createPageFor]);
  const chatTitle = useCallback((sessionId: string) => {
    const session = sessions.find(candidate => candidate.id === sessionId);
    return session ? chatName(session) : undefined;
  }, [sessions]);

  const starts = useRef(new Set<string>());
  const start = async (target: Note, choice: StartChoice): Promise<boolean> => {
    if (starts.current.has(target.id)) return false;
    starts.current.add(target.id);
    setStartingId(target.id);
    try {
      const full = await bridgeApi.readNotePage(target.id);
      const parent = target.parentId ? (page?.note.id === target.parentId ? page.note : byId.get(target.parentId)) : null;
      const brief = taskBrief({ kind: target.kind === "task" ? "task" : "note", title: noteDisplayTitle({ title: target.title, preview: full.markdown }), body: full.markdown, excerpt: target.sourceExcerpt, parentTitle: parent ? noteDisplayTitle(parent) : null });
      const sessionId = await onStartChat({ message: brief, workspaceId: choice.workspaceId, harness: choice.harness });
      if (!sessionId) return false;
      await bridgeApi.updateNote({ id: target.id, addSessionId: sessionId });
      await refreshAll();
      return true;
    } catch (error) {
      fail(error);
      return false;
    } finally {
      starts.current.delete(target.id);
      setStartingId(current => current === target.id ? null : current);
    }
  };

  // A task lives under its page; any other page under a page (one an agent
  // wrote from the open page, say) is nested under it in the list, one an
  // expand/collapse chevron away.
  const visible = tabNotes ?? [];
  // A seeded tab keeps the list's old behaviors (the tree, pending captures,
  // its empty sentence); an edited or new one is a plain flat list.
  const kind = effective ? seededKind({ filter: effective.filter }) : "all";
  const visibleIds = new Set(visible.map(note => note.id));
  const childrenOf = new Map<string, Note[]>();
  for (const note of visible) {
    if (note.kind === "task" || !note.parentId) continue;
    const under = childrenOf.get(note.parentId) ?? [];
    if (!childrenOf.has(note.parentId)) childrenOf.set(note.parentId, under);
    under.push(note);
  }
  // Searched and single-kind views stay flat; a page keeps its rows someplace
  // findable even when its parent is not listed.
  const asTree = !query && (kind === "all" || kind === "pages");
  // The whole chain above the open page, for its breadcrumb: Root > Parent > Here.
  const ancestors = useMemo(() => {
    const chain: Note[] = [];
    let id: string | null = page?.note.parentId ?? null;
    for (let guard = 0; id && byId.has(id) && guard < 64; guard += 1) {
      const holder = byId.get(id)!;
      chain.unshift(holder);
      id = holder.parentId ?? null;
      if (chain.some(item => item.id === id)) break;
    }
    return chain;
  }, [page, byId]);
  // Opening a nested page opens the tree down to its row.
  const ancestorIds = ancestors.map(ancestor => ancestor.id).join(" ");
  useEffect(() => {
    if (ancestorIds) changeExpanded(next => { for (const id of ancestorIds.split(" ")) next.add(id); });
  }, [ancestorIds, changeExpanded]);

  // A tab re-reads when it, or its override, changes.
  const tabKey = JSON.stringify([activeView?.id ?? null, viewDirty ? effective : null]);
  useEffect(() => { void refreshTab(); }, [tabKey, refreshTab]);

  /** Save a tab; resolves to whether it was stored. */
  const saveTab = async (view: NoteDatabaseView, patch: Partial<ReturnType<typeof viewInputOf>>): Promise<boolean> => {
    if (!system) return false;
    try {
      await bridgeApi.saveDatabaseView({ databaseId: system.id, view: { ...viewInputOf(view), ...patch } });
    } catch (error) { fail(error); return false; }
    await rereadSystem();
    await refreshTab();
    return true;
  };
  const hideTab = async (view: NoteDatabaseView) => {
    if (!system || tabViews.length <= 1) return;
    try {
      await bridgeApi.deleteDatabaseView({ databaseId: system.id, viewId: view.id });
      setActiveViewId(null);
      setEditingView(false);
      await rereadSystem();
      await refreshTab();
    } catch (error) { fail(error); }
  };
  // The seven seeded tabs come back exactly: saved anew, then every other
  // view deleted (the database always keeps at least one).
  const resetTabs = async () => {
    if (!system) return;
    try {
      const old = tabViews;
      for (const [order, seed] of SEEDED_VIEWS.entries()) await bridgeApi.saveDatabaseView({ databaseId: system.id, view: { name: seed.name, type: "list", order, filter: seed.filter } });
      for (const view of old) await bridgeApi.deleteDatabaseView({ databaseId: system.id, viewId: view.id });
      for (const view of old) override.reset(view);
      setActiveViewId(null);
      setEditingView(false);
      await rereadSystem();
      await refreshTab();
    } catch (error) { fail(error); }
  };

  return <div className="flex h-full min-h-0 bg-background">
    <aside aria-label="Notes list" className={clsx("min-h-0 w-full shrink-0 flex-col border-r border-border md:w-[18.5rem]", selectedId ? "hidden" : "flex", listOpen ? "md:flex" : "md:hidden")}>
      <div className="flex h-12 shrink-0 items-center gap-1 pl-4 pr-2">
        <h2 className="m-0 flex-1 text-[13px] font-semibold text-foreground">Notes</h2>
        <button type="button" aria-label="Hide the notes list" title={`Hide the notes list (${chordLabel("toggle-notes-list")})`} onClick={toggleList} className="hidden size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:grid"><PanelLeft size={15} aria-hidden="true" /></button>
      </div>
      <NotesCapture focusRequest={(captureRequest ?? 0) + focusCapture} onCapture={capture} />
      <div className="shrink-0 px-3 pt-3">
        <label className="flex h-8 items-center gap-2 rounded-lg bg-muted px-2.5 focus-within:ring-2 focus-within:ring-ring">
          <Search size={13} className="shrink-0 text-faint" aria-hidden="true" />
          <input aria-label="Search notes" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search notes" className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-faint" />
        </label>
        <div role="tablist" aria-label="Filter notes" className="mt-2 flex flex-wrap gap-0.5">
          {tabViews.map(view => {
            const active = view.id === activeView?.id;
            return <span key={view.id} className={clsx("inline-flex items-center rounded-md", active && "bg-accent")}>
              <button type="button" role="tab" aria-selected={active} onClick={() => { setActiveViewId(view.id); setEditingView(false); }} className={clsx("h-7 rounded-md px-2 text-[12.5px] transition-colors", active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}>{view.name}</button>
              {active && system && <TabMenu view={view} last={tabViews.length <= 1} onEdit={() => setEditingView(current => !current)} onRename={name => void saveTab(view, { name })} onHide={() => void hideTab(view)} />}
            </span>;
          })}
        </div>
        {editingView && system && activeView && effective && <div className="mt-2">
          <ViewToolbar
            database={system.database}
            view={activeView}
            query={effective}
            dirty={viewDirty}
            workspaces={workspaces}
            onChange={patch => override.change(activeView, patch)}
            onReset={() => override.reset(activeView)}
            onSave={() => void override.save(activeView, query => saveTab(activeView, { filter: query.filter, sorts: query.sorts, groupBy: query.groupBy }))}
          />
        </div>}
      </div>
      <ul aria-label="Notes" className="m-0 min-h-0 flex-1 list-none space-y-px overflow-y-auto px-2 py-2">
        {(kind === "all" || kind === "inbox") && pending.map(item => <li key={item.key} aria-busy={!item.failed} data-capture={item.failed ? "failed" : "saving"} className="rounded-lg px-2.5 py-2">
          <p className="m-0 truncate text-[13.5px] font-medium text-foreground">{noteDisplayTitle({ title: null, preview: item.body })}</p>
          {item.failed
            ? <div className="mt-1 flex items-center gap-1 text-[12px]">
              <span className="min-w-0 flex-1 truncate text-destructive" title={item.failed}>Couldn't save</span>
              <button type="button" onClick={() => retry(item.key, item.body)} className="rounded-md px-1.5 py-0.5 text-foreground transition-colors hover:bg-accent">Retry</button>
              <button type="button" onClick={() => setPending(current => current.filter(other => other.key !== item.key))} className="rounded-md px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Discard</button>
            </div>
            : <p className="m-0 mt-0.5 text-[12px] text-faint">Saving…</p>}
        </li>)}
        {asTree ? <PageTree
          roots={visible.filter(note => !note.parentId || !visibleIds.has(note.parentId))}
          visibleIds={visibleIds}
          childrenOf={childrenOf}
          expanded={expandedPages}
          activeId={selectedId}
          workspaces={workspaces}
          onToggle={toggleExpand}
          onOpen={id => void open(id)}
        /> : visible.map(note => <li key={note.id}>
          <NoteRow note={note} active={note.id === selectedId} parent={note.parentId ? byId.get(note.parentId) : undefined} project={workspaces.find(workspace => workspace.id === note.workspaceId)?.title} onOpen={() => void open(note.id)} />
        </li>)}
        {tabNotes && !visible.length && !pending.length && <li className="px-3 py-8 text-center text-[12.5px] leading-relaxed text-muted-foreground">
          {query ? "No notes match." : (kind && EMPTY[kind]) ?? "Nothing matches this view."}
        </li>}
      </ul>
      <NotesFooter root={list?.root} onNewPage={() => void createPage()} onNewDatabase={() => void createDatabase()} onResetTabs={system ? () => void resetTabs() : undefined} onError={fail} />
    </aside>

    <main className={clsx("min-h-0 min-w-0 flex-1 overflow-y-auto", selectedId ? "block" : "hidden md:block")}>
      {page ? <NotePageView
        key={page.note.id}
        page={page}
        jobs={list?.jobs.filter(job => job.pageId === page.note.id) ?? []}
        adapters={adapters}
        workspaces={workspaces}
        sessions={sessions}
        defaultHarness={defaultHarness}
        chatTitle={chatTitle}
        // A database's rows are its table, not a list of pages.
        childPages={page.database ? [] : allNotes.filter(note => note.parentId === page.note.id && note.kind !== "task" && !page.markdown.includes(`bridge://p/${note.id}"`))}
        ancestors={ancestors}
        startingId={startingId}
        chatOpen={chatOpen}
        onToggleChat={copilot ? toggleChat : undefined}
        listOpen={listOpen}
        onToggleList={toggleList}
        pageTitle={id => { const child = byId.get(id); return child ? noteDisplayTitle(child) : undefined; }}
        notes={allNotes}
        onOpenByTitle={openByTitle}
        onCreatePage={createPageFor}
        onBack={() => select(null)}
        onOpenPage={id => void open(id)}
        onChanged={() => void refreshAll()}
        onSaved={saved => { if (selected.current === saved.note.id) setPage(saved); void refreshList(); }}
        onStart={start}
        onOpenChat={onOpenChat}
        onDeleted={() => { select(null); void refreshList(); }}
        onError={onError}
      /> : <EmptyPage loading={!!selectedId} chatOpen={chatOpen} onToggleChat={copilot ? toggleChat : undefined} onShowList={listOpen ? undefined : toggleList} onNewPage={() => void createPage()} onCapture={() => setFocusCapture(value => value + 1)} />}
    </main>

    {copilot && chatOpen && <aside aria-label="Notes agent panel" className="hidden min-h-0 w-[24rem] shrink-0 flex-col border-l border-border lg:flex">{copilot.render(page, toggleChat)}</aside>}
  </div>;
}

function NotesCapture({ focusRequest, onCapture }: { focusRequest?: number; onCapture: (body: string) => void }) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { if (focusRequest) box.current?.focus(); }, [focusRequest]);
  const submit = () => {
    const body = text;
    if (!body.trim()) return;
    setText("");
    // Exactly what was typed, cleared at once. A failure stays in the list
    // with Retry, so the box is free for the next thought.
    onCapture(body);
  };
  return <div className="shrink-0 px-3 pt-3">
    <div className="rounded-2xl bg-card px-3 pb-2 pt-2.5 shadow-sm ring-1 ring-border transition-shadow focus-within:ring-2 focus-within:ring-ring">
      <textarea
        ref={box}
        aria-label="Capture a note"
        title={`Capture from anywhere with ${chordLabel("capture-note")}`}
        value={text}
        rows={2}
        placeholder="Capture a thought…"
        onChange={event => setText(event.target.value)}
        onKeyDown={event => {
          if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
          event.preventDefault();
          submit();
        }}
        className="block w-full resize-none bg-transparent text-[14px] leading-relaxed text-foreground outline-none placeholder:text-faint"
      />
      <div className="mt-1 flex items-center justify-between gap-2 text-[11px] text-faint">
        <span>Kept exactly as typed</span>
        <span className="flex items-center gap-1"><Kbd className="h-4 min-w-4 text-[10px]">↵</Kbd>save</span>
      </div>
    </div>
  </div>;
}

function NoteRow({ note, active, parent, project, onOpen }: { note: Note; active: boolean; parent?: Note; project?: string; onOpen: () => void }) {
  const title = noteDisplayTitle(note);
  const preview = plainPreview(note.preview);
  const firstLine = preview.startsWith(title) ? preview.slice(title.length).trim() : preview;
  const harness = agentHarness(note);
  const meta = [parent && `from ${noteDisplayTitle(parent)}`, project, ...note.tags.slice(0, 2).map(tag => `#${tag}`)].filter(Boolean);
  return <button type="button" aria-current={active ? "true" : undefined} onClick={onOpen} className={clsx("block w-full rounded-lg px-2.5 py-2 text-left transition-colors", active ? "bg-selection" : "hover:bg-accent")}>
    <span className="flex min-w-0 items-center gap-1.5">
      {note.kind === "task" && <StatusGlyph state={note.state} size={12} />}
      {note.kind === "database" && <Table2 size={13} aria-label="Database" className="shrink-0 text-muted-foreground" />}
      <span className={clsx("min-w-0 flex-1 truncate text-[13.5px] font-medium", note.state === "done" ? "text-muted-foreground line-through" : "text-foreground")}>{title}</span>
      {harness && <span title={`Written by ${harness}`} className="shrink-0"><HarnessMark harness={harness} size={12} /></span>}
      <span className="shrink-0 text-[11px] tabular-nums text-faint">{relativeTime(note.updatedAt)}</span>
    </span>
    {firstLine && <span className="mt-0.5 block truncate text-[12.5px] text-muted-foreground">{firstLine}</span>}
    {meta.length > 0 && <span className="mt-1 block truncate text-[11px] text-faint">{meta.join(" · ")}</span>}
  </button>;
}

/** The list's nested pages: one row per page, its children indented beneath
 *  it, shown only while the parent is expanded. */
function PageTree({ roots, visibleIds, childrenOf, expanded, activeId, workspaces, onToggle, onOpen, depth = 0 }: {
  roots: Note[];
  visibleIds: Set<string>;
  childrenOf: Map<string, Note[]>;
  expanded: Set<string>;
  activeId: string | null;
  workspaces: Workspace[];
  onToggle: (id: string) => void;
  onOpen: (id: string) => void;
  depth?: number;
}) {
  return <>{roots.map(note => {
    const kids = [...(childrenOf.get(note.id) ?? [])].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    const openNow = expanded.has(note.id);
    return <li key={note.id}>
      <div className="flex items-stretch" style={depth ? { paddingLeft: depth * 14 } : undefined}>
        {kids.length
          ? <button
            type="button"
            aria-label={`Expand the pages under ${noteDisplayTitle(note)}`}
            aria-expanded={openNow}
            title={openNow ? "Hide the pages under it" : "Show the pages under it"}
            onClick={() => onToggle(note.id)}
            className="grid w-5 shrink-0 place-items-center text-faint transition-colors hover:bg-accent hover:text-foreground"
          >
            <ChevronRight size={14} aria-hidden="true" className={clsx("transition-transform", openNow && "rotate-90")} />
          </button>
          : <span aria-hidden="true" className="w-5 shrink-0" />}
        <div className="min-w-0 flex-1">
          <NoteRow note={note} active={note.id === activeId} project={workspaces.find(workspace => workspace.id === note.workspaceId)?.title} onOpen={() => onOpen(note.id)} />
        </div>
      </div>
      {openNow && <ul className="m-0 list-none space-y-px p-0"><PageTree
        roots={kids.filter(child => visibleIds.has(child.id))}
        visibleIds={visibleIds}
        childrenOf={childrenOf}
        expanded={expanded}
        activeId={activeId}
        workspaces={workspaces}
        onToggle={onToggle}
        onOpen={onOpen}
        depth={depth + 1}
      /></ul>}
    </li>;
  })}</>;
}

function NotesFooter({ root, onNewPage, onNewDatabase, onResetTabs, onError }: { root?: string; onNewPage: () => void; onNewDatabase: () => void; onResetTabs?: () => void; onError: (error: unknown) => void }) {
  const [open, setOpen] = useState(false);
  const [settings, setSettings] = useState<NotesSettings | null>(null);
  useEffect(() => {
    if (!open || settings) return;
    void bridgeApi.notesSettings().then(setSettings, onError);
  }, [open, settings, onError]);
  const toggle = async () => {
    if (!settings) return;
    try { setSettings(await bridgeApi.saveNotesSettings({ autoStructure: !settings.autoStructure })); } catch (error) { onError(error); }
  };
  return <div className="flex shrink-0 items-center gap-1 px-2 py-2">
    <button type="button" onClick={onNewPage} className="inline-flex h-8 flex-1 items-center gap-2 rounded-lg px-2 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Plus size={14} aria-hidden="true" />New page</button>
    <button type="button" aria-label="New database" title="New database" onClick={onNewDatabase} className="grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Table2 size={14} aria-hidden="true" /></button>
    <div className="relative">
      <button type="button" aria-label="Notes settings" aria-expanded={open} onClick={() => setOpen(current => !current)} className="grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Settings2 size={14} aria-hidden="true" /></button>
      <Popover open={open} onClose={() => setOpen(false)} label="Notes settings" side="top" align="end" width="w-72">
        <label className="flex cursor-pointer items-start gap-2.5 rounded-lg p-2.5 text-[13px] text-foreground hover:bg-accent">
          <input type="checkbox" checked={settings?.autoStructure ?? false} disabled={!settings} onChange={() => void toggle()} className="mt-0.5 accent-foreground" />
          <span>Auto-structure new captures<span className="mt-0.5 block text-[12px] leading-snug text-muted-foreground">Runs Structure on the default harness after each capture. Off unless you turn it on.</span></span>
        </label>
        {onResetTabs && <button type="button" onClick={() => { setOpen(false); onResetTabs(); }} className="flex h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-foreground hover:bg-accent">Reset tabs to defaults</button>}
        <div className="px-2.5 pb-2 pt-1">
          <p className="m-0 text-[11px] text-muted-foreground">Markdown files</p>
          <p className="m-0 mt-0.5 break-all font-mono text-[11px] text-foreground">{settings?.root ?? root ?? "…"}</p>
        </div>
      </Popover>
    </div>
  </div>;
}

function EmptyPage({ loading, chatOpen, onToggleChat, onShowList, onNewPage, onCapture }: { loading: boolean; chatOpen: boolean; onToggleChat?: () => void; onShowList?: () => void; onNewPage: () => void; onCapture: () => void }) {
  return <div className="flex h-full flex-col">
    <header className="flex h-12 shrink-0 items-center justify-end px-3">
      {onShowList && <button type="button" aria-label="Show the notes list" title={`Show the notes list (${chordLabel("toggle-notes-list")})`} onClick={onShowList} className="mr-auto hidden size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:grid"><PanelLeft size={15} aria-hidden="true" /></button>}
      {onToggleChat && <Pill aria-pressed={chatOpen} active={chatOpen} onClick={onToggleChat}>Ask AI</Pill>}
    </header>
    {loading
      ? <p className="m-0 px-8 text-[13px] text-muted-foreground">Opening…</p>
      : <div className="grid flex-1 place-items-center px-8 pb-24">
        <div className="max-w-sm text-center">
          <h2 className="m-0 text-[1.375rem] font-semibold tracking-[-0.015em] text-foreground">Your words, kept as typed</h2>
          <p className="m-0 mt-2 text-[13.5px] leading-relaxed text-muted-foreground">Capture a thought, or start a page. Structure and tasks run only when you ask, and any chat can write a page here.</p>
          <div className="mt-6 flex items-center justify-center gap-1">
            <PrimaryButton onClick={onNewPage}>New page</PrimaryButton>
            <Pill onClick={onCapture}>Capture <span className="text-faint">{chordLabel("capture-note")}</span></Pill>
          </div>
        </div>
      </div>}
  </div>;
}

/** The open tab's menu: edit its filter, sort and grouping, rename it, or hide it. */
function TabMenu({ view, last, onEdit, onRename, onHide }: { view: NoteDatabaseView; last: boolean; onEdit: () => void; onRename: (name: string) => void; onHide: () => void }) {
  const [open, setOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(view.name);
  const close = () => { setOpen(false); setRenaming(false); };
  const item = "flex h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] text-foreground transition-colors enabled:hover:bg-accent disabled:opacity-40";
  return <span className="relative">
    <button type="button" aria-label={`${view.name} tab menu`} aria-expanded={open} onClick={() => { setName(view.name); setOpen(current => !current); }} className="grid size-6 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground"><MoreHorizontal size={13} aria-hidden="true" /></button>
    <Popover open={open} onClose={close} label={`${view.name} tab`} width="w-56">
      {renaming
        ? <form onSubmit={event => { event.preventDefault(); if (name.trim() && name.trim() !== view.name) onRename(name.trim()); close(); }} className="p-1">
          <input aria-label="Tab name" autoFocus value={name} onChange={event => setName(event.target.value)} className="h-8 w-full rounded-lg bg-muted px-2.5 text-[13px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring" />
        </form>
        : <div role="menu" aria-label={`${view.name} tab actions`}>
          <button type="button" role="menuitem" onClick={() => { close(); onEdit(); }} className={item}>Edit view</button>
          <button type="button" role="menuitem" onClick={() => setRenaming(true)} className={item}>Rename</button>
          <MenuDivider />
          <button type="button" role="menuitem" disabled={last} onClick={() => { close(); onHide(); }} className={item}>{last ? "Hide (the last tab stays)" : "Hide"}</button>
        </div>}
    </Popover>
  </span>;
}
