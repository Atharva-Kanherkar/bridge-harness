// The Notes page's chat rail: one standing `notes` purpose chat docked beside
// the page, built like the Storage copilot. Each message carries the open page
// as a `notes-context` fence, but only when the page changed since the last
// message, so the agent always reads current text and the chat never repeats it.

import { useRef, useState, type ReactNode } from "react";
import { ArrowUp, FileSearch, ListTree, PenLine, Sparkles } from "lucide-react";
import { notesContext, withNotesContext } from "../../notesBrief";
import type { NotePage } from "../../types";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

export interface NotesQuickAsk { label: string; prompt: string; icon: typeof Sparkles; series: string; needsPage: boolean }

/** Starting points, each a whole request. */
export const NOTES_QUICK_ASKS: NotesQuickAsk[] = [
  { label: "Summarize this page", prompt: "Summarize the open page in five bullets. Don't change it.", icon: ListTree, series: "bg-ctx-1", needsPage: true },
  { label: "What's missing?", prompt: "What is missing or unclear on the open page? Ask me; don't edit it.", icon: FileSearch, series: "bg-ctx-2", needsPage: true },
  { label: "Research and write it up", prompt: "Research what the open page is about and write what you find as a new page under it.", icon: PenLine, series: "bg-ctx-3", needsPage: true },
  { label: "Find a note", prompt: "Help me find a note. Ask me what it was about, then search my notes.", icon: Sparkles, series: "bg-ctx-4", needsPage: false },
];

const RAIL = "flex h-full min-h-0 flex-col";

export function NotesCopilot({ page, projectName, starting = false, onAsk, renderChat }: {
  page: NotePage | null;
  projectName?: string | null;
  starting?: boolean;
  /** Ask in the notes chat, starting it if there is none. */
  onAsk: (message: string) => void;
  /** The docked chat, once the notes chat exists. `brief` folds the page on. */
  renderChat?: (brief: (text: string) => string) => ReactNode;
}) {
  const lastContext = useRef("");
  const brief = (text: string) => {
    const context = page ? notesContext({ note: page.note, markdown: page.markdown, projectName }) : "";
    if (!context || context === lastContext.current) return text.trim();
    lastContext.current = context;
    return withNotesContext(text, context);
  };
  const ask = (text: string) => onAsk(brief(text));
  const asks = NOTES_QUICK_ASKS.filter(item => page || !item.needsPage);
  const chat = renderChat?.(brief);

  if (chat) {
    return <div className={clsx(RAIL, "gap-2")}>
      <QuickAsks compact asks={asks} disabled={starting} onAsk={ask} />
      <div className="min-h-0 flex-1">{chat}</div>
    </div>;
  }
  return <NotesCopilotIntro asks={asks} hasPage={!!page} starting={starting} onAsk={ask} />;
}

function QuickAsks({ compact = false, asks, disabled, onAsk }: { compact?: boolean; asks: NotesQuickAsk[]; disabled: boolean; onAsk: (prompt: string) => void }) {
  if (compact) {
    return <ul aria-label="Quick asks" className="m-0 flex shrink-0 list-none gap-1.5 overflow-x-auto p-0 pb-0.5 [scrollbar-width:none]">
      {asks.map(ask => <li key={ask.label} className="shrink-0">
        <button type="button" disabled={disabled} onClick={() => onAsk(ask.prompt)} className="flex items-center gap-1.5 rounded-full border border-border bg-card px-2.5 py-1 text-caption text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">
          <span aria-hidden="true" className={clsx("size-1.5 rounded-full", ask.series)} />{ask.label}
        </button>
      </li>)}
    </ul>;
  }
  return <ul aria-label="Quick asks" className="m-0 grid list-none grid-cols-2 gap-2 p-0 [&>li:last-child:nth-child(odd)]:col-span-2">
    {asks.map(ask => {
      const Icon = ask.icon;
      return <li key={ask.label}>
        <button type="button" disabled={disabled} onClick={() => onAsk(ask.prompt)} className="flex h-full w-full flex-col items-start gap-2 rounded-xl border border-border bg-card p-3 text-left text-caption text-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">
          <span aria-hidden="true" className={clsx("grid size-6 place-items-center rounded-md text-background", ask.series)}><Icon size={13} /></span>
          {ask.label}
        </button>
      </li>;
    })}
  </ul>;
}

function NotesCopilotIntro({ asks, hasPage, starting, onAsk }: { asks: NotesQuickAsk[]; hasPage: boolean; starting: boolean; onAsk: (text: string) => void }) {
  const [question, setQuestion] = useState("");
  const submit = (text: string) => {
    if (!text.trim() || starting) return;
    onAsk(text);
    setQuestion("");
  };
  return <aside aria-label="Notes chat" className={clsx(RAIL, "overflow-hidden rounded-xl border border-border bg-background")}>
    <header className="shrink-0 border-b border-border px-4 py-3">
      <h3 className="m-0 text-ui font-medium text-foreground">Notes agent</h3>
      <p className="m-0 mt-1 text-caption leading-relaxed text-muted-foreground">
        {hasPage ? "Sees the page you have open." : "Open a page and it sees that page too."} It can search your notes, write up research as a page, and add to a page when you ask. Every change it makes is labeled with this chat.
      </p>
    </header>
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      <p className="m-0 mb-2 px-1 text-caption text-muted-foreground">Start with</p>
      <QuickAsks asks={asks} disabled={starting} onAsk={submit} />
    </div>
    <form onSubmit={event => { event.preventDefault(); submit(question); }} className="m-3 mt-0 flex shrink-0 items-end gap-2 rounded-xl border border-border bg-card p-1.5 focus-within:ring-2 focus-within:ring-ring">
      <textarea
        aria-label="Ask about your notes"
        value={question}
        rows={3}
        placeholder="Write up what I know about…"
        onChange={event => setQuestion(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); submit(question); } }}
        className="min-w-0 flex-1 resize-none bg-transparent px-1.5 py-1 text-ui text-foreground outline-none placeholder:text-muted-foreground"
      />
      <button type="submit" aria-label={starting ? "Starting" : "Ask"} disabled={!question.trim() || starting} className="grid size-7 shrink-0 place-items-center rounded-md bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-30">
        <ArrowUp size={14} aria-hidden="true" />
      </button>
    </form>
  </aside>;
}
