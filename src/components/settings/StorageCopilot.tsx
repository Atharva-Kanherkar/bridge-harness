// The Storage page's copilot rail: one standing chat docked beside the
// listing rather than a new chat per question. It is an ordinary direct chat
// (no project, a private scratch dir), so its history survives between visits
// and it stays reachable from the sidebar like any other chat.

import { useState, type ReactNode } from "react";
import { PrimaryButton } from "./kit";

export const STORAGE_QUESTIONS = [
  "What can I safely delete?",
  "Find old node_modules and build folders across my projects",
  "Why is System Data so large?",
  "Clean up developer caches I don't need",
];

/** What the host app lends the Storage page: the page knows what it measured,
 *  the app owns sessions. */
export interface StorageCopilotHost {
  /** Send a prompt into the storage chat, starting the chat if there is none. */
  ask: (prompt: string) => void;
  /** The rail. `brief` wraps a question with what the page measured. */
  render: (brief: (question: string) => string, selectedCount: number) => ReactNode;
}

export function StorageCopilot({ chat, selectedCount, starting = false, onAsk }: {
  /** The docked chat, once the storage chat exists. */
  chat?: ReactNode;
  selectedCount: number;
  starting?: boolean;
  onAsk: (question: string) => void;
}) {
  if (chat) {
    return <div className="h-[min(44rem,calc(100dvh-8rem))] lg:sticky lg:top-6 lg:self-start">{chat}</div>;
  }
  return <StorageCopilotIntro selectedCount={selectedCount} starting={starting} onAsk={onAsk} />;
}

function StorageCopilotIntro({ selectedCount, starting, onAsk }: { selectedCount: number; starting: boolean; onAsk: (question: string) => void }) {
  const [question, setQuestion] = useState("");
  const submit = (text: string) => {
    if (!text.trim() || starting) return;
    onAsk(text);
    setQuestion("");
  };
  return <aside aria-label="Ask Bridge" className="space-y-4 lg:sticky lg:top-6 lg:self-start">
    <div>
      <h3 className="text-ui font-medium text-foreground">Ask Bridge</h3>
      <p className="mt-1 text-caption leading-relaxed text-muted-foreground">
        A chat that stays here and sees what this page measured. It can dig deeper, explain what something is, and clean up once you say yes.
      </p>
    </div>
    <form onSubmit={event => { event.preventDefault(); submit(question); }} className="space-y-2">
      <textarea
        aria-label="Ask about your storage"
        value={question}
        rows={3}
        placeholder="What's using my space?"
        onChange={event => setQuestion(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); submit(question); } }}
        className="w-full resize-none rounded-lg bg-muted/50 px-3 py-2 text-ui text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
      />
      <div className="flex justify-end"><PrimaryButton type="submit" disabled={!question.trim() || starting}>{starting ? "Starting…" : "Ask"}</PrimaryButton></div>
    </form>
    <ul className="space-y-0.5">
      {selectedCount > 0 && <li><button type="button" disabled={starting} onClick={() => submit(`Tell me what the ${selectedCount} item${selectedCount === 1 ? "" : "s"} I selected are, and whether I can delete them.`)} className="w-full rounded-md px-2 py-1.5 text-left text-caption text-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring">Ask about the {selectedCount} selected</button></li>}
      {STORAGE_QUESTIONS.map(text => <li key={text}>
        <button type="button" disabled={starting} onClick={() => submit(text)} className="w-full rounded-md px-2 py-1.5 text-left text-caption text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">{text}</button>
      </li>)}
    </ul>
  </aside>;
}
