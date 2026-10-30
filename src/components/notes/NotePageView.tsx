// One open page: its properties, Your words (the only text a person types and
// the only text background AI never edits), the opt-in Structured view beside
// it, and its tasks. Keyed by page id, so the draft starts fresh per page.

import { useEffect, useState } from "react";
import { Bot, ListChecks, MessageSquarePlus, RefreshCw, Sparkles, Trash2 } from "lucide-react";
import { bridgeApi } from "../../api";
import { actorLabel } from "../../notesBrief";
import { noteDisplayTitle } from "../../notesDraft";
import type { AdapterDescriptor, Note, NoteJob, NotePage, StructuredView, Workspace } from "../../types";
import { cn } from "@/lib/utils";
import { HarnessPicker, StartPicker, type StartChoice } from "./NotePickers";
import { NoteTasks } from "./NoteTasks";
import { useNoteDraft, type DraftStatus } from "./useNoteDraft";

export const NOTE_KINDS = ["note", "idea", "task", "question"];
export const NOTE_STATES = ["inbox", "open", "doing", "done", "archived"];

type Panel = "structure" | "tasks" | "start" | "delete" | null;

const STATUS_COPY: Record<DraftStatus, string> = { saved: "Saved", unsaved: "Unsaved", saving: "Saving…" };
const PASS_COPY = {
  structure: { busy: "Structuring…", failed: "Structure couldn't run", label: "Structure" },
  tasks: { busy: "Finding tasks…", failed: "Turn into tasks couldn't run", label: "Turn into tasks" },
} as const;

const ACTION = "inline-flex h-8 items-center gap-1.5 rounded-lg border border-border px-2.5 text-ui text-foreground transition-colors hover:bg-accent disabled:opacity-50";
const FIELD = "h-7 rounded-md border border-border bg-background px-1.5 text-caption text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";

function latest(jobs: NoteJob[], kind: string): NoteJob | undefined {
  return jobs.filter(job => job.kind === kind).sort((a, b) => b.createdAt.localeCompare(a.createdAt))[0];
}

export interface NotePageViewProps {
  page: NotePage;
  jobs: NoteJob[];
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  defaultHarness: string | null;
  chatTitle: (sessionId: string) => string | undefined;
  /** Set when this page is a task: the page it came from. */
  parent?: Note | null;
  startingId: string | null;
  onOpenPage: (id: string) => void;
  /** Something changed that the list and page should re-read. */
  onChanged: () => void;
  onSaved: (page: NotePage) => void;
  onStart: (target: Note, choice: StartChoice) => Promise<boolean>;
  onOpenChat: (sessionId: string) => void;
  onDeleted: () => void;
  onError: (message: string) => void;
}

export function NotePageView({ page, jobs, adapters, workspaces, defaultHarness, chatTitle, parent, startingId, onOpenPage, onChanged, onSaved, onStart, onOpenChat, onDeleted, onError }: NotePageViewProps) {
  const { note } = page;
  const draft = useNoteDraft(page, onSaved, onError);
  const [view, setView] = useState<"words" | "structured">("words");
  const [panel, setPanel] = useState<Panel>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [title, setTitle] = useState(note.title ?? "");
  const [editingTitle, setEditingTitle] = useState(false);

  useEffect(() => { draft.load(page); }, [page]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (!editingTitle) setTitle(note.title ?? ""); }, [note.title, editingTitle]);

  const update = async (patch: Parameters<typeof bridgeApi.updateNote>[0]) => {
    try { await bridgeApi.updateNote(patch); onChanged(); } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
  };
  const run = async (kind: "structure" | "tasks", harness: string) => {
    setPanel(null);
    draft.flush();
    try {
      if (kind === "structure") { await bridgeApi.structureNote(note.id, harness); setView("structured"); }
      else await bridgeApi.extractNoteTasks(note.id, harness);
      onChanged();
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
  };
  const split = async (index: number) => {
    try {
      const created = await bridgeApi.splitNoteItem(note.id, index);
      setNotice(`Saved "${noteDisplayTitle(created)}" as its own note.`);
      onChanged();
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
  };
  const remove = async () => {
    try { await bridgeApi.deleteNote(note.id); onDeleted(); } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
  };

  const byline = actorLabel(note.lastActor, chatTitle) ?? (note.lastActor === "person" ? null : actorLabel(note.createdBy, chatTitle));
  const structureJob = latest(jobs, "structure");
  const tasksJob = latest(jobs, "tasks");
  const busy = (job?: NoteJob) => job?.status === "pending" || job?.status === "running";
  const knownProject = !note.workspaceId || workspaces.some(workspace => workspace.id === note.workspaceId);

  return <article aria-label={noteDisplayTitle(note)} className="mx-auto w-full max-w-3xl px-6 pb-16 pt-6">
    {parent && <button type="button" onClick={() => onOpenPage(parent.id)} className="mb-2 text-caption text-muted-foreground hover:text-foreground hover:underline">From {noteDisplayTitle(parent)}</button>}
    {byline && <p className="m-0 mb-2 flex items-center gap-1.5 text-caption text-muted-foreground"><Bot size={12} aria-hidden="true" />{byline}</p>}
    <input
      aria-label="Title"
      value={title}
      placeholder={noteDisplayTitle({ title: null, preview: draft.text })}
      onFocus={() => setEditingTitle(true)}
      onChange={event => setTitle(event.target.value)}
      onBlur={() => { setEditingTitle(false); if (title.trim() !== (note.title ?? "")) void update({ id: note.id, title }); }}
      onKeyDown={event => { if (event.key === "Enter") event.currentTarget.blur(); }}
      className="w-full bg-transparent text-[22px] font-semibold tracking-[-0.015em] text-foreground outline-none placeholder:text-muted-foreground/60"
    />

    <div className="mt-3 flex flex-wrap items-center gap-2 text-caption text-muted-foreground">
      <select aria-label="Kind" value={note.kind} onChange={event => void update({ id: note.id, kind: event.target.value })} className={FIELD}>
        {NOTE_KINDS.map(kind => <option key={kind} value={kind}>{kind}</option>)}
      </select>
      <select aria-label="State" value={note.state} onChange={event => void update({ id: note.id, state: event.target.value })} className={FIELD}>
        {NOTE_STATES.map(state => <option key={state} value={state}>{state}</option>)}
      </select>
      <select
        aria-label="Project"
        value={note.workspaceId ?? ""}
        onChange={event => void update(event.target.value ? { id: note.id, workspaceId: event.target.value } : { id: note.id, clearWorkspace: true })}
        className={cn(FIELD, "max-w-56")}
      >
        <option value="">No project</option>
        {workspaces.map(workspace => <option key={workspace.id} value={workspace.id}>{workspace.title}</option>)}
        {!knownProject && <option value={note.workspaceId!}>Unknown project</option>}
      </select>
      {note.tags.map(tag => <span key={tag} className="rounded-full border border-border px-2 py-0.5">#{tag}</span>)}
    </div>

    <div className="mt-4 flex flex-wrap items-center gap-2">
      <button type="button" aria-expanded={panel === "structure"} disabled={busy(structureJob)} onClick={() => setPanel(current => current === "structure" ? null : "structure")} className={ACTION}>
        <Sparkles size={14} aria-hidden="true" />{busy(structureJob) ? PASS_COPY.structure.busy : page.structured ? "Re-structure" : "Structure"}
      </button>
      <button type="button" aria-expanded={panel === "tasks"} disabled={busy(tasksJob)} onClick={() => setPanel(current => current === "tasks" ? null : "tasks")} className={ACTION}>
        <ListChecks size={14} aria-hidden="true" />{busy(tasksJob) ? PASS_COPY.tasks.busy : "Turn into tasks"}
      </button>
      <button type="button" aria-expanded={panel === "start"} onClick={() => setPanel(current => current === "start" ? null : "start")} className={ACTION}>
        <MessageSquarePlus size={14} aria-hidden="true" />Start a chat
      </button>
      {note.sessionIds.length > 0 && <button type="button" onClick={() => onOpenChat(note.sessionIds[note.sessionIds.length - 1])} className={ACTION}>Open chat</button>}
      <span className="flex-1" />
      <button type="button" aria-label="Delete note" onClick={() => setPanel(current => current === "delete" ? null : "delete")} className={cn(ACTION, "border-transparent text-muted-foreground hover:text-foreground")}>
        <Trash2 size={14} aria-hidden="true" />
      </button>
    </div>

    {panel === "structure" && <HarnessPicker label="Structure" what="Builds a second view beside your words: a title, kind, tags, a summary and the separate ideas, each quoting you. Your words stay exactly as they are." adapters={adapters} defaultHarness={defaultHarness} onCancel={() => setPanel(null)} onRun={harness => void run("structure", harness)} />}
    {panel === "tasks" && <HarnessPicker label="Turn into tasks" what="Finds the work this page asks for and adds each piece as a task under it, quoting the words it came from. Existing tasks are kept." adapters={adapters} defaultHarness={defaultHarness} onCancel={() => setPanel(null)} onRun={harness => void run("tasks", harness)} />}
    {panel === "start" && <StartPicker subject="note" adapters={adapters} workspaces={workspaces} defaultHarness={defaultHarness} suggestedWorkspaceId={note.workspaceId ?? parent?.workspaceId ?? null} busy={startingId === note.id} onCancel={() => setPanel(null)} onStart={choice => void onStart(note, choice).then(started => { if (started) setPanel(null); })} />}
    {panel === "delete" && <div role="alertdialog" aria-label="Delete note" className="mt-2 flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card p-3 text-caption">
      <p className="m-0 flex-1 text-foreground">{page.tasks.length ? `Delete this note and its ${page.tasks.length} task${page.tasks.length === 1 ? "" : "s"}?` : "Delete this note?"} Its Markdown file moves to the notes trash folder.</p>
      <button type="button" onClick={() => setPanel(null)} className="inline-flex h-8 items-center rounded-lg px-3 text-ui text-muted-foreground hover:bg-accent hover:text-foreground">Cancel</button>
      <button type="button" onClick={() => void remove()} className="inline-flex h-8 items-center rounded-lg bg-destructive px-3 text-ui font-medium text-destructive-foreground hover:opacity-90">Delete</button>
    </div>}

    {(["structure", "tasks"] as const).map(kind => {
      const job = kind === "structure" ? structureJob : tasksJob;
      return job?.status === "failed" && <p key={kind} role="alert" className="m-0 mt-2 text-caption text-destructive">{PASS_COPY[kind].failed}: {job.detail ?? "it failed without a reason."}</p>;
    })}
    {notice && <p role="status" className="m-0 mt-2 text-caption text-muted-foreground">{notice}</p>}

    <div className="mt-6 flex items-center gap-3 border-b border-border">
      <div role="tablist" aria-label="Views" className="flex gap-1">
        {([["words", "Your words"], ["structured", "Structured"]] as const).map(([id, label]) =>
          <button key={id} type="button" role="tab" aria-selected={view === id} onClick={() => setView(id)} className={cn("-mb-px border-b-2 px-2 pb-2 text-ui transition-colors", view === id ? "border-foreground font-medium text-foreground" : "border-transparent text-muted-foreground hover:text-foreground")}>
            {label}{id === "structured" && page.structured?.stale ? <span aria-label="stale" className="ml-1.5 inline-block size-1.5 rounded-full bg-muted-foreground align-middle" /> : null}
          </button>)}
      </div>
      <span className="flex-1" />
      {view === "words" && <span role="status" aria-label="Save status" className="pb-2 text-caption text-muted-foreground">{STATUS_COPY[draft.status]}</span>}
    </div>

    {view === "words"
      ? <>
        {draft.changedUnderneath && <p role="status" className="m-0 mt-3 rounded-lg bg-accent px-3 py-2 text-caption text-foreground">This page changed while you were typing. Your typing is kept and merges when it saves.</p>}
        <textarea
          aria-label="Your words"
          value={draft.text}
          onChange={event => draft.setText(event.target.value)}
          onBlur={draft.flush}
          spellCheck
          placeholder="Write anything. Markdown works."
          className="mt-3 block min-h-[28rem] w-full resize-y bg-transparent text-[14.5px] leading-relaxed text-foreground outline-none placeholder:text-muted-foreground/60"
        />
      </>
      : <StructuredPanel view={page.structured ?? null} busy={busy(structureJob)} onStructure={() => setPanel("structure")} onSplit={index => void split(index)} />}

    <NoteTasks
      tasks={page.tasks}
      adapters={adapters}
      workspaces={workspaces}
      defaultHarness={defaultHarness}
      pageWorkspaceId={note.workspaceId ?? null}
      startingId={startingId}
      onStart={onStart}
      onOpenChat={onOpenChat}
      onOpen={task => onOpenPage(task.id)}
      onSetState={(task, state) => void update({ id: task.id, state })}
    />
  </article>;
}

function StructuredPanel({ view, busy, onStructure, onSplit }: { view: StructuredView | null; busy: boolean; onStructure: () => void; onSplit: (index: number) => void }) {
  if (!view) {
    return <div className="mt-4 rounded-xl border border-dashed border-border px-5 py-8 text-center">
      <p className="m-0 text-ui font-medium text-foreground">{busy ? "Structuring…" : "No structured view yet"}</p>
      <p className="mx-auto mt-1 max-w-sm text-caption leading-relaxed text-muted-foreground">Structure is opt-in. It builds a second view from your words and never changes them.</p>
      {!busy && <button type="button" onClick={onStructure} className={cn(ACTION, "mt-3")}><Sparkles size={14} aria-hidden="true" />Structure</button>}
    </div>;
  }
  return <section aria-label="Structured view" className="mt-4">
    {view.stale && <div role="status" className="mb-3 flex items-center gap-2 rounded-lg bg-accent px-3 py-2 text-caption text-foreground">
      <span className="flex-1">Your words changed since this was built.</span>
      <button type="button" disabled={busy} onClick={onStructure} className="inline-flex items-center gap-1 font-medium hover:underline disabled:opacity-50"><RefreshCw size={12} aria-hidden="true" />Re-structure</button>
    </div>}
    <h2 className="m-0 text-[17px] font-semibold tracking-[-0.01em] text-foreground">{view.title}</h2>
    <p className="m-0 mt-1 flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
      <span className="rounded-full bg-accent px-2 py-0.5 text-foreground">{view.kind}</span>
      {view.tags.map(tag => <span key={tag}>#{tag}</span>)}
      {view.harness && <span>· by {view.model ?? view.harness}</span>}
    </p>
    {view.summary && <p className="m-0 mt-3 text-ui leading-relaxed text-foreground">{view.summary}</p>}
    {view.items.length > 0 && <ol className="m-0 mt-4 list-none space-y-2 p-0">
      {view.items.map((item, index) => <li key={`${index}-${item.excerpt}`} className="rounded-xl border border-border bg-card p-3">
        <div className="flex items-start gap-2">
          <span className="mt-0.5 shrink-0 rounded-full bg-accent px-2 py-0.5 text-[11px] text-muted-foreground">{item.kind}</span>
          <span className="min-w-0 flex-1 text-ui font-medium text-foreground">{item.title}</span>
          <button type="button" aria-label={`Split "${item.title}" into a note`} onClick={() => onSplit(index)} className="shrink-0 text-caption text-muted-foreground hover:text-foreground hover:underline">Split into note</button>
        </div>
        <blockquote className="m-0 mt-2 whitespace-pre-wrap border-l-2 border-border pl-3 text-caption italic leading-relaxed text-muted-foreground">{item.excerpt}</blockquote>
      </li>)}
    </ol>}
  </section>;
}
