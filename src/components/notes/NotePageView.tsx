// One open page, drawn the way a page reads: a breadcrumb bar, a large title,
// property rows, and Your words (the only text a person types and the only
// text background AI never edits) in a block editor. The opt-in Structured
// view sits beside it, and the page's tasks below; both fill in live while
// their pass streams. Keyed by page id, so the draft starts fresh per page.

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ChevronRight, MoreHorizontal, PanelLeft } from "lucide-react";
import { bridgeApi } from "../../api";
import { relativeTime } from "../../connectorSurface";
import { linkablePages, noteDisplayTitle, plainPreview, quoteText } from "../../notesDraft";
import { streamingStructure, streamingTasks, type StreamingStructure } from "../../notesPass";
import type { AdapterDescriptor, Note, NoteJob, NotePage, NotePassProgressPayload, Session, StructuredView, Workspace } from "../../types";
import { harnessLabel } from "../../utils";
import { chordLabel } from "../../keymap";
import { HarnessMark } from "../harnessMarks";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { PassPicker, PopoverAction, StartPicker, type StartChoice } from "./NotePickers";
import { NoteHistoryDialog } from "./NoteHistoryDialog";
import { NoteTasks } from "./NoteTasks";
import { MenuDivider, Option, OptionList, Pill, Popover, StatusGlyph, useAutoGrow } from "./NotesUi";
import { NoteEditor, type NoteEditorHandle } from "./editor/NoteEditor";
import { useNoteDraft, type DraftStatus } from "./useNoteDraft";

export const NOTE_KINDS = ["note", "idea", "task", "question"];
export const NOTE_STATES = ["inbox", "open", "doing", "done", "archived"];

const capital = (word: string) => word.charAt(0).toUpperCase() + word.slice(1);
const STATUS_COPY: Record<DraftStatus, string> = { saved: "Saved", unsaved: "Unsaved", saving: "Saving…" };
/** "Edited just now", "Edited 5m ago", "Edited Oct 3". */
function editedLabel(iso: string): string {
  const age = relativeTime(iso);
  if (!age || age === "now") return "Edited just now";
  return /^\d+[mhd]$/.test(age) ? `Edited ${age} ago` : `Edited ${age}`;
}
const PASS_COPY = {
  structure: { busy: "Structuring…", failed: "Structure couldn't run" },
  tasks: { busy: "Finding tasks…", failed: "Turn into tasks couldn't run" },
} as const;

function latest(jobs: NoteJob[], kind: string): NoteJob | undefined {
  return jobs.filter(job => job.kind === kind).sort((a, b) => b.createdAt.localeCompare(a.createdAt))[0];
}

export interface NotePageViewProps {
  page: NotePage;
  jobs: NoteJob[];
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  sessions: Session[];
  defaultHarness: string | null;
  chatTitle: (sessionId: string) => string | undefined;
  /** Pages (not tasks) that sit under this one. */
  childPages?: Note[];
  /** The ancestor chain above this page, outermost first. */
  ancestors?: Note[];
  startingId: string | null;
  /** The Notes chat panel, when the host offers one. */
  chatOpen?: boolean;
  onToggleChat?: () => void;
  /** The notes list beside the page, when it can collapse. */
  listOpen?: boolean;
  onToggleList?: () => void;
  /** A page's current title, for child page blocks. */
  pageTitle?: (id: string) => string | undefined;
  /** Every page in the space, for the [[ suggestions. */
  notes?: Note[];
  /** Open the page titled so, making it when none exists. */
  onOpenByTitle?: (title: string) => void;
  /** Make a page: the `/` menu makes one under this one (null title), a
   *  [[ menu makes one named so at the top level. */
  onCreatePage?: (parentId: string | null, title: string | null) => Promise<{ id: string; title: string } | null>;
  onBack: () => void;
  onOpenPage: (id: string) => void;
  /** Something changed that the list and page should re-read. */
  onChanged: () => void;
  onSaved: (page: NotePage) => void;
  onStart: (target: Note, choice: StartChoice) => Promise<boolean>;
  onOpenChat: (sessionId: string) => void;
  onDeleted: () => void;
  onError: (message: string) => void;
}

/** Where a page can be moved: every page besides tasks, itself and its
 *  descendants (a cycle stops the walk). */
function moveTargetsOf(notes: Note[] | undefined, self: string): Note[] {
  const list = notes ?? [];
  const parentOf = new Map(list.map(item => [item.id, item.parentId ?? null]));
  return list.filter(candidate => {
    if (candidate.kind === "task" || candidate.id === self) return false;
    let id: string | null = candidate.parentId ?? null;
    for (let guard = 0; id && guard < 64; guard += 1) {
      if (id === self || id === candidate.id) return false;
      id = parentOf.get(id) ?? null;
    }
    return true;
  });
}

export function NotePageView(props: NotePageViewProps) {
  const { page, jobs, adapters, workspaces, sessions, defaultHarness, chatTitle, childPages = [], ancestors = [], startingId, chatOpen, onToggleChat, listOpen, onToggleList, pageTitle, notes, onOpenByTitle, onCreatePage, onBack, onOpenPage, onChanged, onSaved, onStart, onOpenChat, onDeleted, onError } = props;
  const { note } = page;
  const draft = useNoteDraft(page, onSaved, onError);
  const editor = useRef<NoteEditorHandle>(null);
  const [view, setView] = useState<"words" | "structured">("words");
  const [notice, setNotice] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [progress, setProgress] = useState<Partial<Record<string, NotePassProgressPayload>>>({});

  // Typing the editor has not handed over yet goes to the draft first, so a
  // fresh read never mistakes it for a page with nothing unsaved.
  useEffect(() => { editor.current?.commit(); draft.load(page); }, [page]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void bridgeApi.onNotesPassProgress(payload => {
      if (payload.pageId === note.id) setProgress(current => ({ ...current, [payload.kind]: payload }));
    }).then(stop => { if (alive) unlisten = stop; else stop(); });
    return () => { alive = false; unlisten?.(); };
  }, [note.id]);
  /** Everything typed, stored. Passes, chats and restores read the stored page. */
  const flush = () => { editor.current?.commit(); return draft.flush(); };

  const startersParent = ancestors[ancestors.length - 1];
  const fail = (error: unknown) => onError(error instanceof Error ? error.message : String(error));
  const update = async (patch: Parameters<typeof bridgeApi.updateNote>[0]) => {
    try { await bridgeApi.updateNote(patch); onChanged(); } catch (error) { fail(error); }
  };
  // Passes and chats read the stored page, so everything typed goes first.
  const run = async (kind: "structure" | "tasks", harness: string) => {
    if (!(await flush())) return;
    try {
      if (kind === "structure") { await bridgeApi.structureNote(note.id, harness); setView("structured"); }
      else await bridgeApi.extractNoteTasks(note.id, harness);
      onChanged();
    } catch (error) { fail(error); }
  };
  const split = async (index: number) => {
    try {
      const created = await bridgeApi.splitNoteItem(note.id, index);
      setNotice(`Saved "${noteDisplayTitle(created)}" as its own note.`);
      onChanged();
    } catch (error) { fail(error); }
  };
  // A restore writes over the words, so anything typed is stored first and
  // the restore merges on top of it like any other change.
  const restore = async (seq: number) => {
    if (!(await flush())) return false;
    try { await bridgeApi.restoreNoteVersion(note.id, seq); onChanged(); return true; } catch (error) { fail(error); return false; }
  };
  const startSaved = async (target: Note, choice: StartChoice) => (await flush()) && onStart(target, choice);
  const remove = async () => {
    try { await bridgeApi.deleteNote(note.id); onDeleted(); } catch (error) { fail(error); }
  };

  const structureJob = latest(jobs, "structure");
  const tasksJob = latest(jobs, "tasks");
  const busy = (job?: NoteJob) => job?.status === "pending" || job?.status === "running";
  /** The reply a running pass has streamed so far; none once its job settles. */
  const streamed = (kind: "structure" | "tasks", job?: NoteJob) => {
    const live = progress[kind];
    if (!live) return null;
    const settled = jobs.some(candidate => candidate.id === live.jobId && !busy(candidate));
    const superseded = job && job.id !== live.jobId && busy(job);
    return settled || superseded ? null : live.text;
  };
  const structureText = streamed("structure", structureJob);
  const tasksText = streamed("tasks", tasksJob);
  const moveTargets = useMemo(() => moveTargetsOf(notes, note.id), [notes, note.id]);

  return <div className="flex min-h-full flex-col">
    <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-1 bg-background px-3">
      <nav aria-label="Breadcrumb" className="flex min-w-0 flex-1 items-center gap-0.5 text-[13px]">
        {onToggleList && !listOpen && <button type="button" aria-label="Show the notes list" title={`Show the notes list (${chordLabel("toggle-notes-list")})`} onClick={onToggleList} className="mr-1 hidden size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:grid"><PanelLeft size={15} aria-hidden="true" /></button>}
        <button type="button" onClick={onBack} className="shrink-0 rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Notes</button>
        {ancestors.map(ancestor => <Fragment key={ancestor.id}>
          <ChevronRight size={13} className="shrink-0 text-faint" aria-hidden="true" />
          <button type="button" onClick={() => onOpenPage(ancestor.id)} className="min-w-0 max-w-48 truncate rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">{noteDisplayTitle(ancestor)}</button>
        </Fragment>)}
        <ChevronRight size={13} className="shrink-0 text-faint" aria-hidden="true" />
        <span aria-current="page" className="min-w-0 truncate px-1.5 text-foreground">{noteDisplayTitle({ title: note.title, preview: draft.text || note.preview })}</span>
      </nav>
      <span role="status" aria-label="Save status" className="shrink-0 px-2 text-[12px] text-muted-foreground">{draft.status === "saved" ? editedLabel(note.updatedAt) : STATUS_COPY[draft.status]}</span>
      {onToggleChat && <Pill aria-pressed={!!chatOpen} active={!!chatOpen} onClick={onToggleChat}>Ask AI</Pill>}
      <MoreMenu
        note={note}
        taskCount={page.tasks.length}
        moveTargets={moveTargets}
        onPromote={note.state === "inbox" ? () => void update({ id: note.id, state: "open" }) : undefined}
        onMove={parentId => void update({ id: note.id, ...(parentId ? { parentId } : { clearParent: true }) })}
        onHistory={() => setHistoryOpen(true)}
        onDelete={() => void remove()}
      />
    </header>

    <NoteHistoryDialog page={page} open={historyOpen} chatTitle={chatTitle} onClose={() => setHistoryOpen(false)} onRestore={restore} />

    <article aria-label={noteDisplayTitle(note)} className="mx-auto w-full max-w-[44rem] flex-1 px-8 pb-24 pt-10">
      <TitleField note={note} onCommit={title => void update({ id: note.id, title })} onEnter={() => editor.current?.focus("start")} />

      <dl className="m-0 mt-5">
        <Property label="Status">
          <PropertyMenu label="Status" value={<span className="inline-flex items-center gap-1.5"><StatusGlyph state={note.state} />{capital(note.state)}</span>}>
            {close => <OptionList label="Status">{NOTE_STATES.map(state => <Option key={state} selected={note.state === state} leading={<StatusGlyph state={state} />} onSelect={() => { close(); void update({ id: note.id, state }); }}>{capital(state)}</Option>)}</OptionList>}
          </PropertyMenu>
        </Property>
        <Property label="Type">
          <PropertyMenu label="Type" value={capital(note.kind)}>
            {close => <OptionList label="Type">{NOTE_KINDS.map(kind => <Option key={kind} selected={note.kind === kind} onSelect={() => { close(); void update({ id: note.id, kind }); }}>{capital(kind)}</Option>)}</OptionList>}
          </PropertyMenu>
        </Property>
        <Property label="Project">
          <PropertyMenu label="Project" value={note.workspaceId ? workspaces.find(workspace => workspace.id === note.workspaceId)?.title ?? "Unknown project" : null}>
            {close => <OptionList label="Project" className="max-h-56 overflow-y-auto">
              <Option selected={!note.workspaceId} onSelect={() => { close(); void update({ id: note.id, clearWorkspace: true }); }}>No project</Option>
              {workspaces.map(workspace => <Option key={workspace.id} selected={note.workspaceId === workspace.id} onSelect={() => { close(); void update({ id: note.id, workspaceId: workspace.id }); }}>{workspace.title}</Option>)}
            </OptionList>}
          </PropertyMenu>
        </Property>
        {note.tags.length > 0 && <Property label="Tags">
          <span className="flex flex-wrap gap-1 py-1">{note.tags.map(tag => <span key={tag} className="rounded-md bg-muted px-1.5 py-0.5 text-[12px] text-foreground">{tag}</span>)}</span>
        </Property>}
        <Author note={note} chatTitle={chatTitle} onOpenChat={onOpenChat} />
      </dl>

      <div className="mt-6 flex flex-wrap items-center gap-1 border-b border-border pb-3">
        <div role="tablist" aria-label="Views" className="mr-auto inline-flex rounded-full bg-muted p-0.5">
          {([["words", "Your words"], ["structured", "Structured"]] as const).map(([id, label]) =>
            <button key={id} type="button" role="tab" aria-selected={view === id} onClick={() => setView(id)} className={clsx("inline-flex h-7 items-center gap-1.5 rounded-full px-3 text-[13px] transition-colors", view === id ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:text-foreground")}>
              {label}{id === "structured" && page.structured?.stale && <span aria-label="stale" className="size-1.5 rounded-full bg-warning" />}
            </button>)}
        </div>
        <PopoverAction label={page.structured ? "Re-structure" : "Structure"} triggerLabel="Structure" busy={busy(structureJob)} busyLabel={PASS_COPY.structure.busy} panelLabel="Structure options" align="end">
          {close => <PassPicker label="Structure" what="A second view beside your words: a title, kind, tags, a summary and the separate ideas, each quoting you. Your words stay exactly as they are." adapters={adapters} defaultHarness={defaultHarness} onRun={harness => { close(); void run("structure", harness); }} />}
        </PopoverAction>
        <PopoverAction label="Turn into tasks" busy={busy(tasksJob)} busyLabel={PASS_COPY.tasks.busy} panelLabel="Turn into tasks options" align="end">
          {close => <PassPicker label="Turn into tasks" what="Finds the work this page asks for and adds each piece as a task below, quoting the words it came from. Existing tasks are kept." adapters={adapters} defaultHarness={defaultHarness} onRun={harness => { close(); void run("tasks", harness); }} />}
        </PopoverAction>
        <PopoverAction label="Start a chat" panelLabel="Start a chat" align="end" busy={startingId === note.id} busyLabel="Starting…">
          {close => <StartPicker subject="note" adapters={adapters} workspaces={workspaces} defaultHarness={defaultHarness} suggestedWorkspaceId={note.workspaceId ?? startersParent?.workspaceId ?? null} busy={startingId === note.id} onStart={choice => void startSaved(note, choice).then(started => { if (started) close(); })} />}
        </PopoverAction>
        {note.sessionIds.length > 0 && <Pill onClick={() => onOpenChat(note.sessionIds[note.sessionIds.length - 1])}>Open chat</Pill>}
      </div>

      {(["structure", "tasks"] as const).map(kind => {
        const job = kind === "structure" ? structureJob : tasksJob;
        return job?.status === "failed" && <p key={kind} role="alert" className="m-0 mt-3 text-[13px] text-destructive">{PASS_COPY[kind].failed}: {job.detail ?? "it failed without a reason."}</p>;
      })}
      {notice && <p role="status" className="m-0 mt-3 text-[13px] text-muted-foreground">{notice}</p>}

      {/* Hidden rather than unmounted, so the editor keeps its place and undo. */}
      <div hidden={view !== "words"} className="mt-6">
        {draft.changedUnderneath && <p role="status" className="m-0 mb-4 rounded-lg bg-muted px-3 py-2 text-[13px] text-foreground">This page changed while you were typing. Your typing is kept and merges when it saves.</p>}
        <NoteEditor
          ref={editor}
          text={draft.text}
          title={note.title ?? null}
          onChange={draft.setText}
          onLeave={() => void draft.flush()}
          pages={{
            title: id => pageTitle?.(id),
            open: onOpenPage,
            openByTitle: onOpenByTitle,
            // A blank page has no name to link by; every other one does.
            pages: () => linkablePages(notes ?? []).filter(candidate => candidate.title?.trim() || plainPreview(candidate.preview)).map(candidate => ({ id: candidate.id, title: noteDisplayTitle(candidate) })),
          }}
          onCreatePage={async title => onCreatePage ? (await flush()) ? onCreatePage(title === null ? note.id : null, title) : null : null}
          onError={onError}
        />
      </div>
      {view === "structured" && <StructuredPanel view={page.structured ?? null} busy={busy(structureJob)} streaming={structureText === null ? null : streamingStructure(structureText)} onSplit={index => void split(index)} />}

      {childPages.length > 0 && <section aria-label="Pages" className="mt-12">
        <div className="flex items-baseline gap-2 border-b border-border pb-2">
          <h3 className="m-0 text-[13px] font-semibold text-foreground">Pages</h3>
          <span className="text-[12px] text-muted-foreground">{childPages.length}</span>
        </div>
        <ul className="m-0 list-none divide-y divide-border p-0">
          {childPages.map(child => <li key={child.id}>
            <button type="button" onClick={() => onOpenPage(child.id)} className="flex w-full items-baseline gap-3 py-2.5 text-left">
              <span className="min-w-0 flex-1 truncate text-[14px] text-foreground hover:underline hover:decoration-border hover:underline-offset-4">{noteDisplayTitle(child)}</span>
              <span className="shrink-0 text-[12px] text-faint">{relativeTime(child.updatedAt)}</span>
            </button>
          </li>)}
        </ul>
      </section>}

      <NoteTasks
        tasks={page.tasks}
        drafts={tasksText === null ? [] : streamingTasks(tasksText)}
        adapters={adapters}
        workspaces={workspaces}
        sessions={sessions}
        defaultHarness={defaultHarness}
        pageWorkspaceId={note.workspaceId ?? null}
        startingId={startingId}
        onStart={startSaved}
        onOpenChat={onOpenChat}
        onOpen={task => onOpenPage(task.id)}
        onSetState={(task, state) => void update({ id: task.id, state })}
      />

      {page.backlinks.length > 0 && <section aria-label="Backlinks" className="my-12">
        <div className="flex items-baseline gap-2 border-b border-border pb-2">
          <h3 className="m-0 text-[13px] font-semibold text-foreground">Backlinks</h3>
          <span className="text-[12px] text-muted-foreground">{page.backlinks.length}</span>
        </div>
        <ul className="m-0 list-none divide-y divide-border p-0">
          {page.backlinks.map((backlink, index) => <li key={`${backlink.pageId}-${index}`}>
            <button
              type="button"
              onClick={() => onOpenPage(backlink.pageId)}
              className="flex w-full items-baseline gap-3 py-2.5 text-left transition-colors"
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-baseline gap-2">
                  <span className="min-w-0 truncate text-[14px] text-foreground">{backlink.title || "Untitled"}</span>
                  <span className="shrink-0 rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">{capital(backlink.kind)}</span>
                </span>
                <blockquote className="m-0 mt-0.5 truncate text-[12.5px] text-muted-foreground before:content-['“'] after:content-['”']">{quoteText(backlink.excerpt)}</blockquote>
              </span>
              <span className="shrink-0 text-[12px] text-faint">{relativeTime(backlink.updatedAt)}</span>
            </button>
          </li>)}
        </ul>
      </section>}
    </article>
  </div>;
}

function TitleField({ note, onCommit, onEnter }: { note: Note; onCommit: (title: string) => void; onEnter: () => void }) {
  const [title, setTitle] = useState(note.title ?? "");
  const [editing, setEditing] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { if (!editing) setTitle(note.title ?? ""); }, [note.title, editing]);
  useAutoGrow(box, title);
  return <textarea
    ref={box}
    aria-label="Title"
    rows={1}
    value={title}
    placeholder="Untitled"
    onFocus={() => setEditing(true)}
    onChange={event => setTitle(event.target.value.replace(/\n/g, " "))}
    onBlur={() => { setEditing(false); if (title.trim() !== (note.title ?? "")) onCommit(title); }}
    onKeyDown={event => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.blur(); onEnter(); } }}
    className="block w-full resize-none overflow-hidden bg-transparent text-[2.25rem] font-bold leading-[1.2] tracking-[-0.025em] text-foreground outline-none placeholder:text-foreground/25"
  />;
}

function Property({ label, children }: { label: string; children: ReactNode }) {
  return <div className="flex min-h-8 items-start gap-2">
    <dt className="w-28 shrink-0 py-1.5 text-[13px] text-muted-foreground">{label}</dt>
    <dd className="m-0 min-w-0 flex-1">{children}</dd>
  </div>;
}

/** A property value that opens a menu of choices: plain text until hovered. */
function PropertyMenu({ label, value, children }: { label: string; value: ReactNode | null; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  return <div className="relative inline-block max-w-full">
    <button
      type="button"
      aria-label={label}
      aria-haspopup="listbox"
      aria-expanded={open}
      onClick={() => setOpen(current => !current)}
      className={clsx("-ml-1.5 inline-flex h-8 max-w-full items-center truncate rounded-md px-1.5 text-[13px] transition-colors hover:bg-accent", open && "bg-accent", value === null ? "text-faint" : "text-foreground")}
    >{value ?? "Empty"}</button>
    <Popover open={open} onClose={() => setOpen(false)} label={`${label} choices`} width="w-60">{children(() => setOpen(false))}</Popover>
  </div>;
}

function Author({ note, chatTitle, onOpenChat }: { note: Note; chatTitle: (id: string) => string | undefined; onOpenChat: (id: string) => void }) {
  if (note.lastActor === "file-import") return <Property label="Edited"><span className="inline-block py-1.5 text-[13px] text-foreground">Outside Bridge</span></Property>;
  const actor = /^agent:([^:]+):(.+)$/.exec(note.lastActor) ?? (note.lastActor === "person" ? null : /^agent:([^:]+):(.+)$/.exec(note.createdBy));
  if (!actor) return null;
  const [, harness, sessionId] = actor;
  const title = chatTitle(sessionId);
  return <Property label="Written by">
    <span className="inline-flex max-w-full items-center gap-1.5 py-1.5 text-[13px] text-foreground">
      <HarnessMark harness={harness} size={14} />
      <span className="sr-only">Written by </span>{harnessLabel(harness)}{" "}
      <span className="text-faint">in</span>{" "}
      {title
        ? <button type="button" onClick={() => onOpenChat(sessionId)} className="min-w-0 truncate underline decoration-border underline-offset-4 transition-colors hover:decoration-foreground">{title}</button>
        : <span className="text-muted-foreground">a chat</span>}
    </span>
  </Property>;
}

function MoreMenu({ note, taskCount, moveTargets, onPromote, onMove, onHistory, onDelete }: {
  note: Note;
  taskCount: number;
  moveTargets: Note[];
  onPromote?: () => void;
  onMove?: (parentId: string | null) => void;
  onHistory: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [moving, setMoving] = useState(false);
  const close = () => { setOpen(false); setConfirming(false); setMoving(false); };
  return <div className="relative">
    <button type="button" aria-label="More" aria-expanded={open} onClick={() => open ? close() : setOpen(true)} className="grid size-8 place-items-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <MoreHorizontal size={16} aria-hidden="true" />
    </button>
    <Popover open={open} onClose={close} label="Page actions" align="end" width="w-64">
      {confirming
        ? <div role="alertdialog" aria-label="Delete note" className="p-1.5">
          <p className="m-0 text-[13px] leading-snug text-foreground">{taskCount ? `Delete this note and its ${taskCount} task${taskCount === 1 ? "" : "s"}?` : "Delete this note?"}</p>
          <p className="m-0 mt-1 text-[12px] text-muted-foreground">Its Markdown file moves to the notes trash folder.</p>
          <div className="mt-3 flex justify-end gap-1">
            <Pill onClick={() => setConfirming(false)}>Cancel</Pill>
            <button type="button" onClick={() => { close(); onDelete(); }} className="inline-flex h-8 items-center rounded-full bg-destructive px-3.5 text-[13px] font-medium text-destructive-foreground hover:opacity-90">Delete</button>
          </div>
        </div>
        : moving
          ? <div aria-label="Move this page" className="p-1.5">
            <p className="m-0 px-1 pb-1.5 pt-1 text-[11px] font-medium text-faint">Move {noteDisplayTitle(note)}</p>
            {note.parentId != null && onMove && <MenuButton onClick={() => { onMove(null); close(); }}>Page top level</MenuButton>}
            {moveTargets.map(target => <MenuButton key={target.id} onClick={() => { onMove?.(target.id); close(); }}>{noteDisplayTitle(target)}</MenuButton>)}
            {!moveTargets.length && note.parentId == null && <p className="m-0 px-1.5 py-2 text-[12.5px] text-muted-foreground">No other page to move under.</p>}
          </div>
          : <>
            <MenuButton onClick={() => { close(); onHistory(); }}>History</MenuButton>
            {note.path && <MenuButton onClick={() => { void navigator.clipboard?.writeText(note.path!); close(); }}>Copy file path</MenuButton>}
            {onPromote && <MenuButton onClick={() => { onPromote(); close(); }}>Promote to page</MenuButton>}
            {onMove && <MenuButton onClick={() => setMoving(true)}>Move to…</MenuButton>}
            <MenuDivider />
            <MenuButton destructive onClick={() => setConfirming(true)}>Delete page</MenuButton>
          </>}
    </Popover>
  </div>;
}

function MenuButton({ destructive = false, onClick, children }: { destructive?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" onClick={onClick} className={clsx("flex h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] transition-colors hover:bg-accent", destructive ? "text-destructive" : "text-foreground")}>{children}</button>;
}

function StructuredPanel({ view, busy, streaming, onSplit }: { view: StructuredView | null; busy: boolean; streaming: StreamingStructure | null; onSplit: (index: number) => void }) {
  if (streaming?.title || streaming?.summary) return <StreamingPanel view={streaming} />;
  if (busy) {
    return <div role="status" className="mt-10 text-center">
      <p className="m-0 inline-flex items-center gap-2 text-[15px] font-medium text-foreground"><span aria-hidden="true" className="size-1.5 animate-pulse rounded-full bg-foreground" />Reading your words…</p>
      <p className="mx-auto mt-1.5 max-w-sm text-[13px] leading-relaxed text-muted-foreground">The structured view fills in here as it is written. Your words stay exactly as they are.</p>
    </div>;
  }
  if (!view) {
    return <div className="mt-10 text-center">
      <p className="m-0 text-[15px] font-medium text-foreground">No structured view yet</p>
      <p className="mx-auto mt-1.5 max-w-sm text-[13px] leading-relaxed text-muted-foreground">Structure is opt-in. Click Structure above and it builds a second view from your words, without changing them.</p>
    </div>;
  }
  return <section aria-label="Structured view" className="mt-6">
    {view.stale && <p role="status" className="m-0 mb-5 flex items-center gap-2 rounded-lg bg-muted px-3 py-2 text-[13px] text-foreground">
      <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-hidden="true" />
      Your words changed since this was built. Re-structure to refresh it.
    </p>}
    <h2 className="m-0 text-[1.375rem] font-semibold leading-snug tracking-[-0.015em] text-foreground">{view.title}</h2>
    <p className="m-0 mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
      <span className="text-foreground">{capital(view.kind)}</span>
      {view.tags.map(tag => <span key={tag}>#{tag}</span>)}
      {view.harness && <span className="text-faint">· {view.model ?? view.harness}</span>}
    </p>
    {view.summary && <p className="m-0 mt-4 text-[15px] leading-[1.7] text-foreground">{view.summary}</p>}
    {view.items.length > 0 && <ol className="m-0 mt-6 list-none divide-y divide-border border-t border-border p-0">
      {view.items.map((item, index) => <li key={`${index}-${item.excerpt}`} className="group flex items-start gap-3 py-3">
        <span className="w-14 shrink-0 pt-0.5 text-[12px] text-muted-foreground">{capital(item.kind)}</span>
        <div className="min-w-0 flex-1">
          <p className="m-0 text-[14px] font-medium leading-6 text-foreground">{item.title}</p>
          <blockquote className="m-0 mt-0.5 whitespace-pre-wrap text-[13px] leading-relaxed text-muted-foreground before:content-['“'] after:content-['”']">{quoteText(item.excerpt)}</blockquote>
        </div>
        <button type="button" aria-label={`Split "${item.title}" into a note`} onClick={() => onSplit(index)} className="shrink-0 rounded-full px-2.5 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Split into note</button>
      </li>)}
    </ol>}
  </section>;
}

/** A Structure reply as it is written: each field once it can be read. */
function StreamingPanel({ view }: { view: StreamingStructure }) {
  const caret = <span aria-hidden="true" className="ml-0.5 inline-block h-[1em] w-[2px] translate-y-[2px] animate-pulse bg-foreground" />;
  const writing = !view.items.length;
  return <section aria-label="Structured view" aria-busy="true" className="mt-6">
    <p role="status" className="m-0 mb-4 inline-flex items-center gap-2 text-[12px] text-muted-foreground"><span aria-hidden="true" className="size-1.5 animate-pulse rounded-full bg-foreground" />Structuring…</p>
    {view.title && <h2 className="m-0 text-[1.375rem] font-semibold leading-snug tracking-[-0.015em] text-foreground">{view.title}{!view.summary && caret}</h2>}
    {(view.kind || view.tags.length > 0) && <p className="m-0 mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
      {view.kind && <span className="text-foreground">{capital(view.kind)}</span>}
      {view.tags.map(tag => <span key={tag}>#{tag}</span>)}
    </p>}
    {view.summary && <p className="m-0 mt-4 text-[15px] leading-[1.7] text-foreground">{view.summary}{writing && caret}</p>}
    {view.items.length > 0 && <ol className="m-0 mt-6 list-none divide-y divide-border border-t border-border p-0">
      {view.items.map((item, index) => <li key={index} className="flex items-start gap-3 py-3">
        <span className="w-14 shrink-0 pt-0.5 text-[12px] text-muted-foreground">{capital(item.kind)}</span>
        <div className="min-w-0 flex-1">
          <p className="m-0 text-[14px] font-medium leading-6 text-foreground">{item.title}{index === view.items.length - 1 && caret}</p>
          {item.excerpt && <blockquote className="m-0 mt-0.5 whitespace-pre-wrap text-[13px] leading-relaxed text-muted-foreground before:content-['“'] after:content-['”']">{quoteText(item.excerpt)}</blockquote>}
        </div>
      </li>)}
    </ol>}
  </section>;
}
