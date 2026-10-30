// Bridge Notes: capture in your own words, open any page (yours or one an
// agent wrote), structure it or turn it into tasks when you ask, and start a
// Bridge chat from any task with or without a project.
//
// The list and the open page re-read on every `notes/page_changed`, so a page
// an agent writes appears live; the editor keeps unsaved typing and lets the
// core merge it.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Bot, NotebookPen, Plus, Search, Settings2 } from "lucide-react";
import { bridgeApi } from "../api";
import { taskBrief } from "../notesBrief";
import { noteDisplayTitle } from "../notesDraft";
import type { AdapterDescriptor, Note, NotePage, NotesList, NotesSettings, Session, Workspace } from "../types";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { chordLabel } from "../keymap";
import { chatName } from "./sidebarChats";
import { NotePageView } from "./notes/NotePageView";
import type { StartChoice } from "./notes/NotePickers";

export const JOB_POLL_MS = 1500;

type Filter = "all" | "inbox" | "tasks";
const FILTERS: [Filter, string][] = [["all", "All"], ["inbox", "Inbox"], ["tasks", "Tasks"]];

export interface NoteChatRequest { message: string; workspaceId: string | null; harness: string }

/** What the host lends the docked Notes chat: the open page, as it stands. */
export interface NotesCopilotHost {
  render: (page: NotePage | null) => ReactNode;
}

export interface NotesScreenProps {
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  sessions: Session[];
  /** The chat default harness, preselected in every picker. */
  defaultHarness: string | null;
  /** A new value focuses the capture box (`capture-note`). */
  captureRequest?: number;
  /** Start a Bridge chat with `message` as its first turn; resolves to its id. */
  onStartChat: (request: NoteChatRequest) => Promise<string | undefined>;
  onOpenChat: (sessionId: string) => void;
  onError: (message: string) => void;
  copilot?: NotesCopilotHost;
  /** How often to re-read while a Structure or Tasks pass is queued. */
  pollMs?: number;
}

const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const isAgent = (actor: string) => actor.startsWith("agent:");

export function NotesScreen({ adapters, workspaces, sessions, defaultHarness, captureRequest, onStartChat, onOpenChat, onError, copilot, pollMs = JOB_POLL_MS }: NotesScreenProps) {
  const [list, setList] = useState<NotesList | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [page, setPage] = useState<NotePage | null>(null);
  const [pending, setPending] = useState<{ key: string; body: string }[]>([]);
  const [startingId, setStartingId] = useState<string | null>(null);
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
  const refreshList = useCallback(async () => {
    try { setList(await bridgeApi.listNotes({ query: queryRef.current.trim() || null })); } catch (error) { fail(error); }
  }, [fail]);
  const refreshPage = useCallback(async () => {
    const id = selected.current;
    if (!id) return;
    try {
      const next = await bridgeApi.readNotePage(id);
      if (selected.current === id) setPage(next);
    } catch {
      // Deleted elsewhere: close it.
      if (selected.current === id) select(null);
    }
  }, [select]);
  const refreshAll = useCallback(async () => { await Promise.all([refreshList(), refreshPage()]); }, [refreshList, refreshPage]);

  useEffect(() => {
    const timer = setTimeout(() => void refreshList(), query ? 200 : 0);
    return () => clearTimeout(timer);
  }, [query, refreshList]);

  // Live: every committed change, from anyone, re-reads. Bursts (a pass adding
  // several tasks) coalesce into one read.
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    let burst: ReturnType<typeof setTimeout> | undefined;
    void bridgeApi.onNotesPageChanged(() => {
      clearTimeout(burst);
      burst = setTimeout(() => void refreshAll(), 30);
    }).then(stop => { if (alive) unlisten = stop; else stop(); });
    return () => { alive = false; clearTimeout(burst); unlisten?.(); };
  }, [refreshAll]);

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
    try {
      const next = await bridgeApi.readNotePage(id);
      if (selected.current === id) setPage(next);
    } catch (error) { fail(error); }
  }, [fail, select]);

  const capture = async (body: string) => {
    const key = crypto.randomUUID();
    setPending(current => [{ key, body }, ...current]);
    try {
      await bridgeApi.captureNote(body);
      await refreshList();
      return true;
    } catch (error) {
      fail(error);
      return false;
    } finally {
      setPending(current => current.filter(item => item.key !== key));
    }
  };

  const createPage = async () => {
    try {
      const note = await bridgeApi.createNotePage({ title: null, content: "" });
      await refreshList();
      await open(note.id);
    } catch (error) { fail(error); }
  };

  const notes = list?.notes ?? [];
  const byId = useMemo(() => new Map(notes.map(note => [note.id, note])), [notes]);
  const chatTitle = useCallback((sessionId: string) => {
    const session = sessions.find(candidate => candidate.id === sessionId);
    return session ? chatName(session) : undefined;
  }, [sessions]);

  const start = async (target: Note, choice: StartChoice): Promise<boolean> => {
    setStartingId(target.id);
    try {
      const full = await bridgeApi.readNotePage(target.id);
      const parent = target.parentId ? (page?.note.id === target.parentId ? page.note : byId.get(target.parentId)) : null;
      const brief = taskBrief({ title: noteDisplayTitle({ title: target.title, preview: full.markdown }), body: full.markdown, excerpt: target.sourceExcerpt, parentTitle: parent ? noteDisplayTitle(parent) : null });
      const sessionId = await onStartChat({ message: brief, workspaceId: choice.workspaceId, harness: choice.harness });
      if (!sessionId) return false;
      await bridgeApi.updateNote({ id: target.id, addSessionId: sessionId, ...(target.kind === "task" && target.state === "open" ? { state: "doing" } : {}) });
      await refreshAll();
      return true;
    } catch (error) {
      fail(error);
      return false;
    } finally {
      setStartingId(null);
    }
  };

  const visible = notes.filter(note => filter === "tasks" ? note.kind === "task" : !note.parentId && (filter === "all" || note.state === "inbox"));
  const parent = page?.note.parentId ? byId.get(page.note.parentId) ?? null : null;

  return <div className="flex h-full min-h-0">
    <aside aria-label="Notes list" className={clsx("min-h-0 w-full shrink-0 flex-col border-r border-border md:flex md:w-80", selectedId ? "hidden" : "flex")}>
      <NotesCapture focusRequest={captureRequest} onCapture={capture} />
      <div className="flex shrink-0 items-center gap-1 px-3 pb-2">
        <div role="tablist" aria-label="Filter notes" className="flex gap-0.5 rounded-lg bg-muted p-0.5">
          {FILTERS.map(([id, label]) => <button key={id} type="button" role="tab" aria-selected={filter === id} onClick={() => setFilter(id)} className={clsx("rounded-md px-2 py-0.5 text-caption transition-colors", filter === id ? "bg-background font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground")}>{label}</button>)}
        </div>
        <label className="ml-auto flex min-w-0 flex-1 items-center gap-1 rounded-lg border border-border px-2 focus-within:ring-2 focus-within:ring-ring">
          <Search size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <input aria-label="Search notes" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search" className="h-7 min-w-0 flex-1 bg-transparent text-caption text-foreground outline-none placeholder:text-muted-foreground" />
        </label>
      </div>
      <ul aria-label="Notes" className="m-0 min-h-0 flex-1 list-none space-y-0.5 overflow-y-auto px-2 pb-2">
        {filter !== "tasks" && pending.map(item => <li key={item.key} aria-busy="true" className="rounded-lg px-2.5 py-2 opacity-70">
          <p className="m-0 truncate text-ui font-medium text-foreground">{noteDisplayTitle({ title: null, preview: item.body })}</p>
          <p className="m-0 text-caption text-muted-foreground">Saving…</p>
        </li>)}
        {visible.map(note => <li key={note.id}>
          <NoteRow note={note} active={note.id === selectedId} parent={note.parentId ? byId.get(note.parentId) : undefined} project={workspaces.find(workspace => workspace.id === note.workspaceId)?.title} onOpen={() => void open(note.id)} />
        </li>)}
        {list && !visible.length && !pending.length && <li className="px-3 py-6 text-center text-caption text-muted-foreground">
          {query ? "No notes match." : filter === "tasks" ? "No tasks yet. Open a page and click Turn into tasks." : filter === "inbox" ? "The Inbox is empty." : "Nothing here yet. Type above and press Enter."}
        </li>}
      </ul>
      <NotesFooter root={list?.root} onNewPage={() => void createPage()} onError={fail} />
    </aside>

    <main className={clsx("min-h-0 min-w-0 flex-1 overflow-y-auto", selectedId ? "block" : "hidden md:block")}>
      {selectedId && <button type="button" onClick={() => select(null)} className="m-3 mb-0 text-caption text-muted-foreground hover:text-foreground md:hidden">All notes</button>}
      {page ? <NotePageView
        key={page.note.id}
        page={page}
        jobs={list?.jobs.filter(job => job.pageId === page.note.id) ?? []}
        adapters={adapters}
        workspaces={workspaces}
        defaultHarness={defaultHarness}
        chatTitle={chatTitle}
        parent={parent}
        startingId={startingId}
        onOpenPage={id => void open(id)}
        onChanged={() => void refreshAll()}
        onSaved={saved => { if (selected.current === saved.note.id) setPage(saved); void refreshList(); }}
        onStart={start}
        onOpenChat={onOpenChat}
        onDeleted={() => { select(null); void refreshList(); }}
        onError={onError}
      /> : <EmptyPage loading={!!selectedId} />}
    </main>

    {copilot && <div className="hidden w-[22rem] shrink-0 border-l border-border p-3 xl:block">{copilot.render(page)}</div>}
  </div>;
}

function NotesCapture({ focusRequest, onCapture }: { focusRequest?: number; onCapture: (body: string) => Promise<boolean> }) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { if (focusRequest) box.current?.focus(); }, [focusRequest]);
  const submit = () => {
    const body = text;
    if (!body.trim()) return;
    setText("");
    // Exactly what was typed, cleared at once; a failure hands it back.
    void onCapture(body).then(saved => { if (!saved) setText(current => current || body); });
  };
  return <div className="shrink-0 p-3">
    <div className="rounded-xl border border-border bg-card p-2 focus-within:ring-2 focus-within:ring-ring">
      <textarea
        ref={box}
        aria-label="Capture a note"
        value={text}
        rows={3}
        placeholder="Dump a thought. Enter saves it."
        onChange={event => setText(event.target.value)}
        onKeyDown={event => {
          if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
          event.preventDefault();
          submit();
        }}
        className="block w-full resize-none bg-transparent px-1 text-ui leading-relaxed text-foreground outline-none placeholder:text-muted-foreground"
      />
    </div>
    <p className="m-0 mt-1.5 px-1 text-[11px] text-muted-foreground">Saved exactly as typed. Shift+Enter for a new line · {chordLabel("capture-note")} from anywhere</p>
  </div>;
}

function NoteRow({ note, active, parent, project, onOpen }: { note: Note; active: boolean; parent?: Note; project?: string; onOpen: () => void }) {
  const title = noteDisplayTitle(note);
  const preview = note.preview.replace(/^#+\s*/, "").replace(/\s+/g, " ").trim();
  return <button type="button" aria-current={active ? "true" : undefined} onClick={onOpen} className={clsx("block w-full rounded-lg px-2.5 py-2 text-left transition-colors", active ? "bg-selection text-selection-foreground" : "hover:bg-accent")}>
    <span className="flex min-w-0 items-center gap-1.5">
      {(isAgent(note.createdBy) || isAgent(note.lastActor)) && <Bot size={12} className="shrink-0 text-muted-foreground" aria-label="Agent" />}
      <span className={clsx("min-w-0 flex-1 truncate text-ui font-medium", note.state === "done" && "text-muted-foreground line-through")}>{title}</span>
      {note.kind !== "note" && <span className="shrink-0 text-[11px] text-muted-foreground">{note.kind}</span>}
    </span>
    {preview && preview !== title && <span className="mt-0.5 line-clamp-2 text-caption leading-snug text-muted-foreground">{preview}</span>}
    {(parent || project || note.tags.length > 0) && <span className="mt-1 flex min-w-0 flex-wrap gap-1 text-[11px] text-muted-foreground">
      {parent && <span className="truncate">from {noteDisplayTitle(parent)}</span>}
      {project && <span className="truncate rounded-full border border-border px-1.5">{project}</span>}
      {note.tags.slice(0, 3).map(tag => <span key={tag}>#{tag}</span>)}
    </span>}
  </button>;
}

function NotesFooter({ root, onNewPage, onError }: { root?: string; onNewPage: () => void; onError: (error: unknown) => void }) {
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
  return <div className="shrink-0 border-t border-border p-2">
    {open && <div role="group" aria-label="Notes settings" className="mb-2 rounded-lg border border-border bg-card p-3 text-caption">
      <label className="flex items-start gap-2 text-foreground">
        <input type="checkbox" checked={settings?.autoStructure ?? false} disabled={!settings} onChange={() => void toggle()} className="mt-0.5" />
        <span>Auto-structure new captures<span className="block text-muted-foreground">Runs Structure on the default harness after each capture. Off unless you turn it on.</span></span>
      </label>
      <p className="m-0 mt-2 text-muted-foreground">Markdown files live in</p>
      <p className="m-0 break-all font-mono text-[11px] text-foreground">{settings?.root ?? root ?? "…"}</p>
    </div>}
    <div className="flex items-center gap-1">
      <button type="button" onClick={onNewPage} className="inline-flex h-8 flex-1 items-center gap-1.5 rounded-lg px-2 text-ui text-foreground transition-colors hover:bg-accent"><Plus size={14} aria-hidden="true" />New page</button>
      <button type="button" aria-label="Notes settings" aria-expanded={open} onClick={() => setOpen(current => !current)} className="grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Settings2 size={14} aria-hidden="true" /></button>
    </div>
  </div>;
}

function EmptyPage({ loading }: { loading: boolean }) {
  if (loading) return <p className="m-0 p-8 text-caption text-muted-foreground">Opening…</p>;
  return <div className="grid h-full place-items-center p-8">
    <div className="max-w-sm text-center">
      <NotebookPen size={20} strokeWidth={1.5} className="mx-auto mb-3 text-muted-foreground" aria-hidden="true" />
      <p className="m-0 text-ui font-medium text-foreground">Your words, kept as typed</p>
      <p className="m-0 mt-1 text-caption leading-relaxed text-muted-foreground">Capture on the left, or open a page. Structure and Turn into tasks run only when you click them. Any chat can write a page here when you ask it to.</p>
    </div>
  </div>;
}
