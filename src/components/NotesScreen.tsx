// Bridge Notes: capture in your own words, open any page (yours or one an
// agent wrote), structure it or turn it into tasks when you ask, and start a
// Bridge chat from any task with or without a project.
//
// The list and the open page re-read on every `notes/page_changed`, so a page
// an agent writes appears live; the editor keeps unsaved typing and lets the
// core merge it.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Plus, Search, Settings2 } from "lucide-react";
import { bridgeApi } from "../api";
import { taskBrief } from "../notesBrief";
import { noteDisplayTitle, plainPreview } from "../notesDraft";
import type { AdapterDescriptor, Note, NotePage, NotesList, NotesSettings, Session, Workspace } from "../types";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { chordLabel } from "../keymap";
import { relativeTime } from "../connectorSurface";
import { Kbd } from "./ui/kbd";
import { HarnessMark } from "./harnessMarks";
import { Pill, Popover, PrimaryButton, StatusGlyph } from "./notes/NotesUi";
import { chatName } from "./sidebarChats";
import { NotePageView } from "./notes/NotePageView";
import type { StartChoice } from "./notes/NotePickers";

export const JOB_POLL_MS = 1500;

type Filter = "all" | "inbox" | "tasks" | "archived";
const FILTERS: [Filter, string][] = [["all", "All"], ["inbox", "Inbox"], ["tasks", "Tasks"], ["archived", "Archived"]];

export interface NoteChatRequest { message: string; workspaceId: string | null; harness: string }

/** What the host lends the Notes chat panel: the open page, as it stands,
 *  and a way to close the panel. */
export interface NotesCopilotHost {
  render: (page: NotePage | null, onClose: () => void) => ReactNode;
}

const CHAT_OPEN_KEY = "bridge.notes.chatOpen";
const readChatOpen = () => { try { return localStorage.getItem(CHAT_OPEN_KEY) === "1"; } catch { return false; } };

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
/** The harness that wrote a page, when an agent did. */
const agentHarness = (note: Note) => /^agent:([^:]+):/.exec(note.lastActor)?.[1] ?? /^agent:([^:]+):/.exec(note.createdBy)?.[1];

export function NotesScreen({ adapters, workspaces, sessions, defaultHarness, captureRequest, onStartChat, onOpenChat, onError, copilot, pollMs = JOB_POLL_MS }: NotesScreenProps) {
  const [list, setList] = useState<NotesList | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
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
  const refreshList = useCallback(async () => {
    const ticket = ++listRequest.current;
    try {
      // Archived pages come too, for the Archived tab; the others leave them out.
      const next = await bridgeApi.listNotes({ query: queryRef.current.trim() || null, includeArchived: true });
      if (ticket === listRequest.current) setList(next);
    } catch (error) {
      if (ticket === listRequest.current) fail(error);
    }
  }, [fail]);
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
      const brief = taskBrief({ kind: target.kind === "task" ? "task" : "note", title: noteDisplayTitle({ title: target.title, preview: full.markdown }), body: full.markdown, excerpt: target.sourceExcerpt, parentTitle: parent ? noteDisplayTitle(parent) : null });
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

  // A task lives under its page; any other page under a page (one an agent
  // wrote from the open page, say) is listed like a top-level one.
  const childTask = (note: Note) => !!note.parentId && note.kind === "task";
  const visible = notes.filter(note => {
    if (filter === "archived") return note.state === "archived";
    if (note.state === "archived") return false;
    return filter === "tasks" ? note.kind === "task" : !childTask(note) && (filter === "all" || note.state === "inbox");
  });
  const parent = page?.note.parentId ? byId.get(page.note.parentId) ?? null : null;

  return <div className="flex h-full min-h-0 bg-background">
    <aside aria-label="Notes list" className={clsx("min-h-0 w-full shrink-0 flex-col border-r border-border md:flex md:w-[18.5rem]", selectedId ? "hidden" : "flex")}>
      <NotesCapture focusRequest={(captureRequest ?? 0) + focusCapture} onCapture={capture} />
      <div className="shrink-0 px-3 pt-3">
        <label className="flex h-8 items-center gap-2 rounded-lg bg-muted px-2.5 focus-within:ring-2 focus-within:ring-ring">
          <Search size={13} className="shrink-0 text-faint" aria-hidden="true" />
          <input aria-label="Search notes" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search notes" className="min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-faint" />
        </label>
        <div role="tablist" aria-label="Filter notes" className="mt-2 flex gap-0.5">
          {FILTERS.map(([id, label]) => <button key={id} type="button" role="tab" aria-selected={filter === id} onClick={() => setFilter(id)} className={clsx("h-7 rounded-md px-2 text-[12.5px] transition-colors", filter === id ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}>{label}</button>)}
        </div>
      </div>
      <ul aria-label="Notes" className="m-0 min-h-0 flex-1 list-none space-y-px overflow-y-auto px-2 py-2">
        {(filter === "all" || filter === "inbox") && pending.map(item => <li key={item.key} aria-busy={!item.failed} data-capture={item.failed ? "failed" : "saving"} className="rounded-lg px-2.5 py-2">
          <p className="m-0 truncate text-[13.5px] font-medium text-foreground">{noteDisplayTitle({ title: null, preview: item.body })}</p>
          {item.failed
            ? <div className="mt-1 flex items-center gap-1 text-[12px]">
              <span className="min-w-0 flex-1 truncate text-destructive" title={item.failed}>Couldn't save</span>
              <button type="button" onClick={() => retry(item.key, item.body)} className="rounded-md px-1.5 py-0.5 text-foreground transition-colors hover:bg-accent">Retry</button>
              <button type="button" onClick={() => setPending(current => current.filter(other => other.key !== item.key))} className="rounded-md px-1.5 py-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Discard</button>
            </div>
            : <p className="m-0 mt-0.5 text-[12px] text-faint">Saving…</p>}
        </li>)}
        {visible.map(note => <li key={note.id}>
          <NoteRow note={note} active={note.id === selectedId} parent={note.parentId ? byId.get(note.parentId) : undefined} project={workspaces.find(workspace => workspace.id === note.workspaceId)?.title} onOpen={() => void open(note.id)} />
        </li>)}
        {list && !visible.length && !pending.length && <li className="px-3 py-8 text-center text-[12.5px] leading-relaxed text-muted-foreground">
          {query ? "No notes match." : filter === "tasks" ? "No tasks yet. Open a page and choose Turn into tasks." : filter === "inbox" ? "The Inbox is empty." : filter === "archived" ? "Nothing archived. Set a page's status to Archived to put it here." : "Nothing here yet. Type above and press Enter."}
        </li>}
      </ul>
      <NotesFooter root={list?.root} onNewPage={() => void createPage()} onError={fail} />
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
        parent={parent}
        childPages={notes.filter(note => note.parentId === page.note.id && note.kind !== "task")}
        startingId={startingId}
        chatOpen={chatOpen}
        onToggleChat={copilot ? toggleChat : undefined}
        onBack={() => select(null)}
        onOpenPage={id => void open(id)}
        onChanged={() => void refreshAll()}
        onSaved={saved => { if (selected.current === saved.note.id) setPage(saved); void refreshList(); }}
        onStart={start}
        onOpenChat={onOpenChat}
        onDeleted={() => { select(null); void refreshList(); }}
        onError={onError}
      /> : <EmptyPage loading={!!selectedId} chatOpen={chatOpen} onToggleChat={copilot ? toggleChat : undefined} onNewPage={() => void createPage()} onCapture={() => setFocusCapture(value => value + 1)} />}
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
      <span className={clsx("min-w-0 flex-1 truncate text-[13.5px] font-medium", note.state === "done" ? "text-muted-foreground line-through" : "text-foreground")}>{title}</span>
      {harness && <span title={`Written by ${harness}`} className="shrink-0"><HarnessMark harness={harness} size={12} /></span>}
      <span className="shrink-0 text-[11px] tabular-nums text-faint">{relativeTime(note.updatedAt)}</span>
    </span>
    {firstLine && <span className="mt-0.5 block truncate text-[12.5px] text-muted-foreground">{firstLine}</span>}
    {meta.length > 0 && <span className="mt-1 block truncate text-[11px] text-faint">{meta.join(" · ")}</span>}
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
  return <div className="flex shrink-0 items-center gap-1 px-2 py-2">
    <button type="button" onClick={onNewPage} className="inline-flex h-8 flex-1 items-center gap-2 rounded-lg px-2 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Plus size={14} aria-hidden="true" />New page</button>
    <div className="relative">
      <button type="button" aria-label="Notes settings" aria-expanded={open} onClick={() => setOpen(current => !current)} className="grid size-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Settings2 size={14} aria-hidden="true" /></button>
      <Popover open={open} onClose={() => setOpen(false)} label="Notes settings" side="top" align="end" width="w-72">
        <label className="flex cursor-pointer items-start gap-2.5 rounded-lg p-2.5 text-[13px] text-foreground hover:bg-accent">
          <input type="checkbox" checked={settings?.autoStructure ?? false} disabled={!settings} onChange={() => void toggle()} className="mt-0.5 accent-foreground" />
          <span>Auto-structure new captures<span className="mt-0.5 block text-[12px] leading-snug text-muted-foreground">Runs Structure on the default harness after each capture. Off unless you turn it on.</span></span>
        </label>
        <div className="px-2.5 pb-2 pt-1">
          <p className="m-0 text-[11px] text-muted-foreground">Markdown files</p>
          <p className="m-0 mt-0.5 break-all font-mono text-[11px] text-foreground">{settings?.root ?? root ?? "…"}</p>
        </div>
      </Popover>
    </div>
  </div>;
}

function EmptyPage({ loading, chatOpen, onToggleChat, onNewPage, onCapture }: { loading: boolean; chatOpen: boolean; onToggleChat?: () => void; onNewPage: () => void; onCapture: () => void }) {
  return <div className="flex h-full flex-col">
    <header className="flex h-12 shrink-0 items-center justify-end px-3">
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
