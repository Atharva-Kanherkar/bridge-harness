// A page's tasks, drawn as a to-do list: what Turn into tasks found, each
// quoting the words it came from, each a click away from a Bridge chat.
// Starting asks where the chat goes; "No project" is always a choice. While
// Turn into tasks runs, the tasks it is writing show as drafts.

import { noteDisplayTitle, quoteText } from "../../notesDraft";
import { latestTaskChat, noteTaskStatus } from "../../notesTask";
import type { AdapterDescriptor, Note, Session, Workspace } from "../../types";
import { HarnessMark } from "../harnessMarks";
import { PopoverAction, StartPicker, type StartChoice } from "./NotePickers";
import { StatusGlyph } from "./NotesUi";
import { Button } from "@/components/ui/button";

export function NoteTasks({ tasks, drafts = [], adapters, workspaces, sessions, defaultHarness, pageWorkspaceId, startingId, onStart, onOpenChat, onSetState, onOpen }: {
  tasks: Note[];
  /** Tasks a running Turn into tasks is writing, not stored yet. */
  drafts?: { title: string; excerpt: string }[];
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  sessions: Session[];
  defaultHarness: string | null;
  pageWorkspaceId: string | null;
  startingId: string | null;
  onStart: (task: Note, choice: StartChoice) => Promise<boolean>;
  onOpenChat: (sessionId: string) => void;
  onSetState: (task: Note, state: string) => void;
  onOpen: (task: Note) => void;
}) {
  if (tasks.length === 0 && drafts.length === 0) return null;
  const open = tasks.filter(task => task.state !== "done" && task.state !== "archived").length;
  const chats = new Map(sessions.map(session => [session.id, session]));
  // A draft the page already has (a rerun) is not shown twice.
  const fresh = drafts.filter(draft => !tasks.some(task => task.sourceExcerpt === draft.excerpt || noteDisplayTitle(task) === draft.title));
  return <section aria-label="Tasks" className="mt-12">
    <div className="flex items-baseline gap-2 border-b border-border pb-2">
      <h3 className="m-0 text-ui font-semibold text-foreground">Tasks</h3>
      <span className="text-meta text-muted-foreground">{fresh.length ? "Finding tasks…" : open ? `${open} of ${tasks.length} open` : tasks.every(task => task.state === "done") ? "All done" : "No open tasks"}</span>
    </div>
    <ul className="m-0 list-none divide-y divide-border p-0">
      {tasks.map(task => {
        const title = noteDisplayTitle(task);
        const done = task.state === "done";
        const { id: lastChat, session } = latestTaskChat(task, chats);
        const status = startingId === task.id ? "Starting…" : noteTaskStatus(task, session);
        return <li key={task.id} aria-label={title} className="group flex items-start gap-3 py-2.5">
          <Button variant="quiet" size="icon-3xs" className="mt-[3px] shrink-0" tooltip={done ? "Reopen" : "Mark done"}
           
            aria-label={done ? `Reopen ${title}` : `Mark ${title} done`}
           
            onClick={() => onSetState(task, done ? (lastChat ? "doing" : "open") : "done")}
           
          >
            <StatusGlyph state={task.state} size={15} />
          </Button>
          <div className="min-w-0 flex-1">
            <button type="button" onClick={() => onOpen(task)} className={`block max-w-full truncate text-left text-body leading-6 hover:underline hover:decoration-border hover:underline-offset-4 ${done ? "text-muted-foreground line-through" : "text-foreground"}`}>{title}</button>
            <p role="status" className={`m-0 mb-0.5 text-meta ${!done && session?.status === "failed" ? "text-destructive" : "text-muted-foreground"}`}>{status}</p>
            {task.sourceExcerpt && <blockquote className="m-0 line-clamp-2 text-meta leading-snug text-muted-foreground before:content-['“'] after:content-['”']">{quoteText(task.sourceExcerpt)}</blockquote>}
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {lastChat && <Button variant="quiet" size="xs" onClick={() => onOpenChat(lastChat)}>
              {session && <HarnessMark harness={session.harness} size={12} />}Open chat
            </Button>}
            {!lastChat && !done && task.state !== "archived" && <PopoverAction label="Start" triggerLabel={`Start ${title}`} panelLabel="Start a chat" align="end" busy={startingId === task.id} busyLabel="Starting…">
              {close => <StartPicker
                adapters={adapters}
                workspaces={workspaces}
                defaultHarness={defaultHarness}
                suggestedWorkspaceId={task.workspaceId ?? pageWorkspaceId}
                busy={startingId === task.id}
                onStart={choice => void onStart(task, choice).then(started => { if (started) close(); })}
              />}
            </PopoverAction>}
          </div>
        </li>;
      })}
      {fresh.map((draft, index) => <li key={`draft-${index}`} aria-label={`Draft: ${draft.title}`} data-draft="" className="flex items-start gap-3 py-2.5">
        <span aria-hidden="true" className="mt-[7px] grid size-5 shrink-0 place-items-center"><span className="size-2 animate-pulse rounded-full bg-faint" /></span>
        <div className="min-w-0 flex-1">
          <p className="m-0 truncate text-body leading-6 text-muted-foreground">{draft.title}</p>
          {draft.excerpt && <blockquote className="m-0 line-clamp-2 text-meta leading-snug text-faint before:content-['“'] after:content-['”']">{quoteText(draft.excerpt)}</blockquote>}
        </div>
      </li>)}
    </ul>
  </section>;
}
