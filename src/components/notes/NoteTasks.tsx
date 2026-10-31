// A page's tasks, drawn as a to-do list: what Turn into tasks found, each
// quoting the words it came from, each a click away from a Bridge chat.
// Starting asks where the chat goes; "No project" is always a choice.

import { noteDisplayTitle, quoteText } from "../../notesDraft";
import type { AdapterDescriptor, Note, Session, Workspace } from "../../types";
import { HarnessMark } from "../harnessMarks";
import { PopoverAction, StartPicker, type StartChoice } from "./NotePickers";
import { StatusGlyph } from "./NotesUi";

export function NoteTasks({ tasks, adapters, workspaces, sessions, defaultHarness, pageWorkspaceId, startingId, onStart, onOpenChat, onSetState, onOpen }: {
  tasks: Note[];
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
  if (tasks.length === 0) return null;
  const open = tasks.filter(task => task.state !== "done").length;
  return <section aria-label="Tasks" className="mt-12">
    <div className="flex items-baseline gap-2 border-b border-border pb-2">
      <h3 className="m-0 text-[13px] font-semibold text-foreground">Tasks</h3>
      <span className="text-[12px] text-muted-foreground">{open ? `${open} of ${tasks.length} open` : "All done"}</span>
    </div>
    <ul className="m-0 list-none divide-y divide-border p-0">
      {tasks.map(task => {
        const title = noteDisplayTitle(task);
        const done = task.state === "done";
        const lastChat = task.sessionIds[task.sessionIds.length - 1];
        const chatHarness = lastChat ? sessions.find(session => session.id === lastChat)?.harness : undefined;
        return <li key={task.id} aria-label={title} className="group flex items-start gap-3 py-2.5">
          <button
            type="button"
            aria-label={done ? `Reopen ${title}` : `Mark ${title} done`}
            title={done ? "Reopen" : "Mark done"}
            onClick={() => onSetState(task, done ? "open" : "done")}
            className="mt-[3px] grid size-5 shrink-0 place-items-center rounded-full transition-transform hover:scale-110"
          >
            <StatusGlyph state={task.state} size={15} />
          </button>
          <div className="min-w-0 flex-1">
            <button type="button" onClick={() => onOpen(task)} className={`block max-w-full truncate text-left text-[14px] leading-6 hover:underline hover:decoration-border hover:underline-offset-4 ${done ? "text-muted-foreground line-through" : "text-foreground"}`}>{title}</button>
            {task.sourceExcerpt && <blockquote className="m-0 line-clamp-2 text-[12.5px] leading-snug text-muted-foreground before:content-['“'] after:content-['”']">{quoteText(task.sourceExcerpt)}</blockquote>}
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {lastChat && <button type="button" onClick={() => onOpenChat(lastChat)} className="inline-flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[12.5px] text-foreground transition-colors hover:bg-accent">
              {chatHarness && <HarnessMark harness={chatHarness} size={12} />}Open chat
            </button>}
            <PopoverAction label="Start" triggerLabel={`Start ${title}`} panelLabel="Start a chat" align="end" busy={startingId === task.id} busyLabel="Starting…">
              {close => <StartPicker
                adapters={adapters}
                workspaces={workspaces}
                defaultHarness={defaultHarness}
                suggestedWorkspaceId={task.workspaceId ?? pageWorkspaceId}
                busy={startingId === task.id}
                onStart={choice => void onStart(task, choice).then(started => { if (started) close(); })}
              />}
            </PopoverAction>
          </div>
        </li>;
      })}
    </ul>
  </section>;
}
