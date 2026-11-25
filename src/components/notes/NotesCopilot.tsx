// The Notes page's AI panel: one standing `notes` purpose chat, opened from
// "Ask AI" beside the page. Each message carries the open page as a
// `notes-context` fence, but only when the page changed since the last message
// that was delivered, so the agent always reads current text and the chat never
// repeats it. A send that fails or is refused carries the page again next time.

import { useRef, useState, type ReactNode } from "react";
import { ArrowUp, X } from "lucide-react";
import { notesContext, withNotesContext } from "../../notesBrief";
import type { NotePage } from "../../types";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";
import { Textarea } from "@/components/ui/textarea";

export interface NotesQuickAsk { label: string; prompt: string; needsPage: boolean; needsDrawing?: boolean }

/** Starting points, each a whole request. */
export const NOTES_QUICK_ASKS: NotesQuickAsk[] = [
  { label: "Act on the drawing", prompt: "Read the drawing on the open page. Say in two lines what it shows and what I marked. If it asks for something, do it; if it lays out a choice, decide and say why; otherwise propose next steps. Add to the drawing with notes_draw when that makes the answer clearer, and leave what I drew alone.", needsPage: true, needsDrawing: true },
  { label: "Summarize this page", prompt: "Summarize the open page in five bullets. Don't change it.", needsPage: true },
  { label: "What's missing?", prompt: "What is missing or unclear on the open page? Ask me; don't edit it.", needsPage: true },
  { label: "Research and write it up", prompt: "Research what the open page is about and write what you find as a new page under it.", needsPage: true },
  { label: "Find a note", prompt: "Help me find a note. Ask me what it was about, then search my notes.", needsPage: false },
];

export function NotesCopilot({ page, projectName, starting = false, onAsk, onClose, renderChat }: {
  page: NotePage | null;
  projectName?: string | null;
  starting?: boolean;
  /** Ask in the notes chat, starting it if there is none. Resolves to whether
   *  the message was handed to the chat. */
  onAsk: (message: string) => Promise<boolean>;
  onClose?: () => void;
  /** The chat, once the notes chat exists. `send` folds the page on and calls
   *  `deliver` with the message; a rejection leaves the page unsent. */
  renderChat?: (send: (text: string, deliver: (message: string) => Promise<unknown>) => Promise<void>) => ReactNode;
}) {
  const lastContext = useRef("");
  const send = async (text: string, deliver: (message: string) => Promise<unknown>) => {
    const context = page ? notesContext({ note: page.note, markdown: page.markdown, projectName }) : "";
    const fold = !!context && context !== lastContext.current;
    await deliver(fold ? withNotesContext(text, context) : text.trim());
    if (fold) lastContext.current = context;
  };
  const ask = (text: string) => void send(text, async message => {
    if (!(await onAsk(message))) throw new Error("not sent");
  }).catch(() => undefined);
  const drawn = !!page && /^ {0,3}(`{3,}|~{3,})[ \t]*excalidraw\b/im.test(page.markdown);
  const asks = NOTES_QUICK_ASKS.filter(item => (page || !item.needsPage) && (drawn || !item.needsDrawing));
  const chat = renderChat?.(send);

  return <div className="flex h-full min-h-0 flex-col">
    <header className="flex h-12 shrink-0 items-center gap-2 px-4">
      <h3 className="m-0 flex-1 text-ui font-semibold text-foreground">Notes agent</h3>
      {onClose && <Button variant="quiet" size="icon-xs" aria-label="Close the Notes agent" onClick={onClose}><X size={15} aria-hidden="true" /></Button>}
    </header>
    {chat
      ? <div className="flex min-h-0 flex-1 flex-col gap-2 px-3 pb-3">
        <ul aria-label="Quick asks" className="m-0 flex shrink-0 list-none gap-1 overflow-x-auto p-0 [scrollbar-width:none]">
          {asks.map(item => <li key={item.label} className="shrink-0">
            <Button variant="text" size="xs" disabled={starting} onClick={() => ask(item.prompt)}>{item.label}</Button>
          </li>)}
        </ul>
        <div className="min-h-0 flex-1">{chat}</div>
      </div>
      : <NotesCopilotIntro asks={asks} hasPage={!!page} starting={starting} onAsk={ask} />}
  </div>;
}

function NotesCopilotIntro({ asks, hasPage, starting, onAsk }: { asks: NotesQuickAsk[]; hasPage: boolean; starting: boolean; onAsk: (text: string) => void }) {
  const [question, setQuestion] = useState("");
  const submit = (text: string) => {
    if (!text.trim() || starting) return;
    onAsk(text);
    setQuestion("");
  };
  return <aside aria-label="Notes chat" className="flex min-h-0 flex-1 flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto px-4 pt-6">
      <p className="m-0 text-lg font-semibold tracking-snug text-foreground">{hasPage ? "Ask about this page" : "Ask about your notes"}</p>
      <p className="m-0 mt-1.5 text-ui leading-relaxed text-muted-foreground">
        {hasPage ? "It reads the page you have open." : "Open a page and it reads that page too."} It can search your notes, write research up as a page, and add to a page when you ask. Every change it makes is labeled with this chat.
      </p>
      <ul aria-label="Quick asks" className="m-0 mt-5 list-none space-y-0.5 p-0">
        {asks.map(item => <li key={item.label}>
          <RowButton size="bare" tone="default" shape="card" disabled={starting} onClick={() => submit(item.prompt)}><span className="group flex w-full items-center justify-between gap-3 px-3 py-2.5 text-ui">
            {item.label}
            <ArrowUp size={14} aria-hidden="true" className="shrink-0 rotate-45 text-faint transition-colors group-hover:text-foreground" />
          </span></RowButton>
        </li>)}
      </ul>
    </div>
    <form onSubmit={event => { event.preventDefault(); submit(question); }} className="m-3 flex shrink-0 items-end gap-2 rounded-2xl bg-card p-2 text-ui shadow-sm ring-1 ring-border focus-within:ring-2 focus-within:ring-ring">
      <Textarea unstyled size="bare"
        aria-label="Ask about your notes"
        value={question}
        rows={2}
        placeholder="Write up what I know about…"
        onChange={event => setQuestion(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); submit(question); } }}
        className="mx-1.5 my-1 min-w-0 flex-1"
      />
      <Button variant="default" size="icon-sm" className="shrink-0" type="submit" aria-label={starting ? "Starting" : "Ask"} disabled={!question.trim() || starting}>
        <ArrowUp size={15} aria-hidden="true" />
      </Button>
    </form>
  </aside>;
}
