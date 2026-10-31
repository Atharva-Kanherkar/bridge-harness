// One open page, drawn the way a page reads: a breadcrumb bar, a large title,
// property rows, and Your words (the only text a person types and the only
// text background AI never edits) rendered as a page until you click into it.
// The opt-in Structured view sits beside it, and the page's tasks below.
// Keyed by page id, so the draft starts fresh per page.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronRight, MoreHorizontal } from "lucide-react";
import { bridgeApi } from "../../api";
import { relativeTime } from "../../connectorSurface";
import { noteDisplayTitle, quoteText } from "../../notesDraft";
import type { AdapterDescriptor, Note, NoteJob, NotePage, Session, StructuredView, Workspace } from "../../types";
import { harnessLabel } from "../../utils";
import { HarnessMark } from "../harnessMarks";
import { Markdown } from "../Markdown";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { PassPicker, PopoverAction, StartPicker, type StartChoice } from "./NotePickers";
import { NoteTasks } from "./NoteTasks";
import { MenuDivider, Option, OptionList, Pill, Popover, StatusGlyph, useAutoGrow } from "./NotesUi";
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
  /** Set when this page is a task: the page it came from. */
  parent?: Note | null;
  startingId: string | null;
  /** The Notes chat panel, when the host offers one. */
  chatOpen?: boolean;
  onToggleChat?: () => void;
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

export function NotePageView(props: NotePageViewProps) {
  const { page, jobs, adapters, workspaces, sessions, defaultHarness, chatTitle, parent, startingId, chatOpen, onToggleChat, onBack, onOpenPage, onChanged, onSaved, onStart, onOpenChat, onDeleted, onError } = props;
  const { note } = page;
  const draft = useNoteDraft(page, onSaved, onError);
  const [view, setView] = useState<"words" | "structured">("words");
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => { draft.load(page); }, [page]); // eslint-disable-line react-hooks/exhaustive-deps

  const fail = (error: unknown) => onError(error instanceof Error ? error.message : String(error));
  const update = async (patch: Parameters<typeof bridgeApi.updateNote>[0]) => {
    try { await bridgeApi.updateNote(patch); onChanged(); } catch (error) { fail(error); }
  };
  // Passes and chats read the stored page, so everything typed goes first.
  const run = async (kind: "structure" | "tasks", harness: string) => {
    if (!(await draft.flush())) return;
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
  const startSaved = async (target: Note, choice: StartChoice) => (await draft.flush()) && onStart(target, choice);
  const remove = async () => {
    try { await bridgeApi.deleteNote(note.id); onDeleted(); } catch (error) { fail(error); }
  };

  const structureJob = latest(jobs, "structure");
  const tasksJob = latest(jobs, "tasks");
  const busy = (job?: NoteJob) => job?.status === "pending" || job?.status === "running";

  return <div className="flex min-h-full flex-col">
    <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-1 bg-background px-3">
      <nav aria-label="Breadcrumb" className="flex min-w-0 flex-1 items-center gap-0.5 text-[13px]">
        <button type="button" onClick={onBack} className="shrink-0 rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Notes</button>
        {parent && <>
          <ChevronRight size={13} className="shrink-0 text-faint" aria-hidden="true" />
          <button type="button" onClick={() => onOpenPage(parent.id)} className="min-w-0 max-w-48 truncate rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">{noteDisplayTitle(parent)}</button>
        </>}
        <ChevronRight size={13} className="shrink-0 text-faint" aria-hidden="true" />
        <span aria-current="page" className="min-w-0 truncate px-1.5 text-foreground">{noteDisplayTitle({ title: note.title, preview: draft.text || note.preview })}</span>
      </nav>
      <span role="status" aria-label="Save status" className="shrink-0 px-2 text-[12px] text-muted-foreground">{draft.status === "saved" ? editedLabel(note.updatedAt) : STATUS_COPY[draft.status]}</span>
      {onToggleChat && <Pill aria-pressed={!!chatOpen} active={!!chatOpen} onClick={onToggleChat}>Ask AI</Pill>}
      <MoreMenu note={note} taskCount={page.tasks.length} onDelete={() => void remove()} />
    </header>

    <article aria-label={noteDisplayTitle(note)} className="mx-auto w-full max-w-[44rem] flex-1 px-8 pb-24 pt-10">
      <TitleField note={note} onCommit={title => void update({ id: note.id, title })} />

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
          {close => <StartPicker subject="note" adapters={adapters} workspaces={workspaces} defaultHarness={defaultHarness} suggestedWorkspaceId={note.workspaceId ?? parent?.workspaceId ?? null} busy={startingId === note.id} onStart={choice => void startSaved(note, choice).then(started => { if (started) close(); })} />}
        </PopoverAction>
        {note.sessionIds.length > 0 && <Pill onClick={() => onOpenChat(note.sessionIds[note.sessionIds.length - 1])}>Open chat</Pill>}
      </div>

      {(["structure", "tasks"] as const).map(kind => {
        const job = kind === "structure" ? structureJob : tasksJob;
        return job?.status === "failed" && <p key={kind} role="alert" className="m-0 mt-3 text-[13px] text-destructive">{PASS_COPY[kind].failed}: {job.detail ?? "it failed without a reason."}</p>;
      })}
      {notice && <p role="status" className="m-0 mt-3 text-[13px] text-muted-foreground">{notice}</p>}

      {view === "words"
        ? <YourWords title={note.title} text={draft.text} changedUnderneath={draft.changedUnderneath} onChange={draft.setText} onLeave={() => void draft.flush()} />
        : <StructuredPanel view={page.structured ?? null} busy={busy(structureJob)} onSplit={index => void split(index)} />}

      <NoteTasks
        tasks={page.tasks}
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
    </article>
  </div>;
}

function TitleField({ note, onCommit }: { note: Note; onCommit: (title: string) => void }) {
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
    onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); } }}
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

function MoreMenu({ note, taskCount, onDelete }: { note: Note; taskCount: number; onDelete: () => void }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const close = () => { setOpen(false); setConfirming(false); };
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
        : <>
          {note.path && <MenuButton onClick={() => { void navigator.clipboard?.writeText(note.path!); close(); }}>Copy file path</MenuButton>}
          {note.path && <MenuDivider />}
          <MenuButton destructive onClick={() => setConfirming(true)}>Delete page</MenuButton>
        </>}
    </Popover>
  </div>;
}

function MenuButton({ destructive = false, onClick, children }: { destructive?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" onClick={onClick} className={clsx("flex h-8 w-full items-center rounded-lg px-2.5 text-left text-[13px] transition-colors hover:bg-accent", destructive ? "text-destructive" : "text-foreground")}>{children}</button>;
}

/** Your words: the rendered page until clicked, then its Markdown in place. */
function YourWords({ title, text, changedUnderneath, onChange, onLeave }: { title?: string | null; text: string; changedUnderneath: boolean; onChange: (text: string) => void; onLeave: () => void }) {
  const [editing, setEditing] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const writing = editing || !text.trim();
  useAutoGrow(box, text, writing);
  useEffect(() => {
    const field = box.current;
    if (!editing || !field) return;
    field.focus();
    field.setSelectionRange(field.value.length, field.value.length);
  }, [editing]);
  // A page that opens with its own title as a heading would say it twice.
  const heading = /^#\s+(.+)\n*/.exec(text);
  const shown = heading && title && heading[1].trim() === title.trim() ? text.slice(heading[0].length) : text;
  return <div className="mt-6">
    {changedUnderneath && <p role="status" className="m-0 mb-4 rounded-lg bg-muted px-3 py-2 text-[13px] text-foreground">This page changed while you were typing. Your typing is kept and merges when it saves.</p>}
    {writing
      ? <textarea
        ref={box}
        aria-label="Your words"
        value={text}
        onChange={event => onChange(event.target.value)}
        onBlur={() => { onLeave(); setEditing(false); }}
        spellCheck
        placeholder="Write anything. Markdown works."
        className="block min-h-32 w-full resize-none overflow-hidden bg-transparent text-[15px] leading-[1.75] text-foreground outline-none placeholder:text-faint"
      />
      : <div
        role="button"
        tabIndex={0}
        aria-label="Edit your words"
        title="Click to edit"
        onClick={event => { if (!(event.target as HTMLElement).closest("a,button")) setEditing(true); }}
        onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); setEditing(true); } }}
        className="note-md -mx-3 cursor-text rounded-xl px-3 py-1 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Markdown text={shown} />
      </div>}
  </div>;
}

function StructuredPanel({ view, busy, onSplit }: { view: StructuredView | null; busy: boolean; onSplit: (index: number) => void }) {
  if (!view) {
    return <div className="mt-10 text-center">
      <p className="m-0 text-[15px] font-medium text-foreground">{busy ? "Structuring…" : "No structured view yet"}</p>
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
