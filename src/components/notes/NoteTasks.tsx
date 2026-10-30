// A page's tasks: what Turn into tasks found, each quoting the words it came
// from, each a click away from a Bridge chat. Starting asks where the chat
// goes; "No project" is always a choice.

import { useState } from "react";
import { MessageSquare, Play } from "lucide-react";
import { noteDisplayTitle } from "../../notesDraft";
import type { AdapterDescriptor, Note, Workspace } from "../../types";
import { StartPicker, type StartChoice } from "./NotePickers";

export const TASK_STATES = ["open", "doing", "done"] as const;

export function NoteTasks({ tasks, adapters, workspaces, defaultHarness, pageWorkspaceId, startingId, onStart, onOpenChat, onSetState, onOpen }: {
  tasks: Note[];
  adapters: AdapterDescriptor[];
  workspaces: Workspace[];
  defaultHarness: string | null;
  pageWorkspaceId: string | null;
  startingId: string | null;
  onStart: (task: Note, choice: StartChoice) => Promise<boolean>;
  onOpenChat: (sessionId: string) => void;
  onSetState: (task: Note, state: string) => void;
  onOpen: (task: Note) => void;
}) {
  const [picking, setPicking] = useState<string | null>(null);
  if (tasks.length === 0) return null;
  return <section aria-label="Tasks" className="mt-8">
    <h3 className="m-0 mb-2 text-caption font-medium uppercase tracking-wide text-muted-foreground">Tasks <span className="tabular-nums">{tasks.length}</span></h3>
    <ul className="m-0 list-none space-y-2 p-0">
      {tasks.map(task => {
        const title = noteDisplayTitle(task);
        const lastChat = task.sessionIds[task.sessionIds.length - 1];
        return <li key={task.id} aria-label={title} className="rounded-xl border border-border bg-card p-3">
          <div className="flex min-w-0 items-start gap-2">
            <select
              aria-label={`State of ${title}`}
              value={TASK_STATES.includes(task.state as typeof TASK_STATES[number]) ? task.state : "open"}
              onChange={event => onSetState(task, event.target.value)}
              className="h-7 shrink-0 rounded-md border border-border bg-background px-1.5 text-caption text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {TASK_STATES.map(state => <option key={state} value={state}>{state}</option>)}
            </select>
            <button type="button" onClick={() => onOpen(task)} className={`min-w-0 flex-1 truncate pt-1 text-left text-ui font-medium hover:underline ${task.state === "done" ? "text-muted-foreground line-through" : "text-foreground"}`}>{title}</button>
            {lastChat && <button type="button" onClick={() => onOpenChat(lastChat)} className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-border px-2 text-caption text-foreground transition-colors hover:bg-accent">
              <MessageSquare size={12} aria-hidden="true" />Open chat
            </button>}
            <button type="button" aria-label={`Start ${title}`} onClick={() => setPicking(current => current === task.id ? null : task.id)} className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md bg-primary px-2 text-caption font-medium text-primary-foreground transition-opacity hover:opacity-90">
              <Play size={11} aria-hidden="true" />Start
            </button>
          </div>
          {task.sourceExcerpt && <blockquote className="m-0 mt-2 border-l-2 border-border pl-3 text-caption italic leading-relaxed text-muted-foreground whitespace-pre-wrap">{task.sourceExcerpt}</blockquote>}
          {picking === task.id && <StartPicker
            adapters={adapters}
            workspaces={workspaces}
            defaultHarness={defaultHarness}
            suggestedWorkspaceId={task.workspaceId ?? pageWorkspaceId}
            busy={startingId === task.id}
            onCancel={() => setPicking(null)}
            onStart={choice => void onStart(task, choice).then(started => { if (started) setPicking(null); })}
          />}
        </li>;
      })}
    </ul>
  </section>;
}
