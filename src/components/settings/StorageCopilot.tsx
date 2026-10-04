// The Storage page's copilot rail: one standing chat docked beside the
// listing rather than a new chat per question. It is a direct chat created
// with the `storage` purpose, so its brief is a system prompt of its own
// (`storage_agent.rs`), its history survives between visits, and it stays
// reachable from the sidebar like any other chat.

import { useState, type ReactNode } from "react";
import { ArrowUp, Boxes, Container, FolderSearch, HardDrive, ListChecks, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";

export interface StorageQuickAsk { label: string; prompt: string; icon: typeof Sparkles; series: string }

/** Starting points, each a full instruction so a click is a whole request. */
export const STORAGE_QUICK_ASKS: StorageQuickAsk[] = [
  { label: "What can I safely delete?", prompt: "What can I safely delete? Measure first, then give me a plan.", icon: Sparkles, series: "bg-ctx-1" },
  { label: "Old node_modules and builds", prompt: "Find node_modules, target, dist, and .next folders across my projects that haven't been touched in 30 days, and plan their removal.", icon: FolderSearch, series: "bg-ctx-2" },
  { label: "Developer caches", prompt: "Clean up developer caches I don't need: Homebrew, npm, bun, pnpm, Go, Cargo, Xcode DerivedData. Prefer each tool's own cleanup command.", icon: Boxes, series: "bg-ctx-3" },
  { label: "Docker and simulators", prompt: "How much are Docker and the iOS/Android simulators using, and what can go?", icon: Container, series: "bg-ctx-4" },
  { label: "Why is System Data so large?", prompt: "Why is System Data so large? Check local Time Machine snapshots, caches, and logs.", icon: HardDrive, series: "bg-ctx-5" },
];

/** What the host app lends the Storage page: the page knows what it measured,
 *  the app owns sessions. */
export interface StorageCopilotHost {
  /** Send a prompt into the storage chat, starting the chat if there is none. */
  ask: (prompt: string) => void;
  /** The rail. `brief` folds what the page measured onto a message. */
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
    return <div className={cn(RAIL, "gap-2")}>
      <QuickAsks compact selectedCount={selectedCount} disabled={starting} onAsk={onAsk} />
      <div className="min-h-0 flex-1">{chat}</div>
    </div>;
  }
  return <StorageCopilotIntro selectedCount={selectedCount} starting={starting} onAsk={onAsk} />;
}

/** The rail fills the window beside the page (title bar, sticky offset, and
 *  bottom gap taken out) and stays put while the listing scrolls. */
const RAIL = "flex h-[38rem] flex-col lg:sticky lg:top-6 lg:h-[calc(100dvh-6rem)] lg:self-start";

const selectedAsk = (count: number) => `Tell me what the ${count} item${count === 1 ? "" : "s"} I selected are, and whether I can delete them.`;

/** One-click requests. Compact above a live chat (a scrolling chip row); a
 *  grid of tiles before the chat exists. */
function QuickAsks({ compact = false, selectedCount, disabled, onAsk }: { compact?: boolean; selectedCount: number; disabled: boolean; onAsk: (prompt: string) => void }) {
  const asks = selectedCount > 0
    ? [{ label: `Explain the ${selectedCount} selected`, prompt: selectedAsk(selectedCount), icon: ListChecks, series: "bg-ctx-6" }, ...STORAGE_QUICK_ASKS]
    : STORAGE_QUICK_ASKS;
  if (compact) {
    return <ul aria-label="Quick asks" className="flex shrink-0 gap-1.5 overflow-x-auto pb-0.5 [scrollbar-width:none]">
      {asks.map(ask => <li key={ask.label} className="shrink-0">
        <button type="button" disabled={disabled} onClick={() => onAsk(ask.prompt)} className="flex items-center gap-1.5 rounded-full border border-border bg-card px-2.5 py-1 text-caption text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">
          <span aria-hidden="true" className={cn("size-1.5 rounded-full", ask.series)} />{ask.label}
        </button>
      </li>)}
    </ul>;
  }
  return <ul aria-label="Quick asks" className="grid grid-cols-2 gap-2 [&>li:last-child:nth-child(odd)]:col-span-2">
    {asks.map(ask => {
      const Icon = ask.icon;
      return <li key={ask.label}>
        <button type="button" disabled={disabled} onClick={() => onAsk(ask.prompt)} className="flex h-full w-full flex-col items-start gap-2 rounded-xl border border-border bg-card p-3 text-left text-caption text-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">
          <span aria-hidden="true" className={cn("grid size-6 place-items-center rounded-md text-background", ask.series)}><Icon size={13} /></span>
          {ask.label}
        </button>
      </li>;
    })}
  </ul>;
}

function StorageCopilotIntro({ selectedCount, starting, onAsk }: { selectedCount: number; starting: boolean; onAsk: (question: string) => void }) {
  const [question, setQuestion] = useState("");
  const submit = (text: string) => {
    if (!text.trim() || starting) return;
    onAsk(text);
    setQuestion("");
  };
  // Shaped like the chat it becomes: what it can do on top, the starting
  // points in the middle, the composer pinned to the bottom.
  return <aside aria-label="Ask Bridge" className={cn(RAIL, "overflow-hidden rounded-xl border border-border bg-background")}>
    <header className="shrink-0 border-b border-border px-4 py-3">
      <h3 className="text-ui font-medium text-foreground">Storage agent</h3>
      <p className="mt-1 text-caption leading-relaxed text-muted-foreground">
        Sees what this page measured, digs deeper with read-only commands, and proposes cleanups as plans you tick and approve. Nothing moves without you.
      </p>
    </header>
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      <p className="mb-2 px-1 text-caption text-muted-foreground">Start with</p>
      <QuickAsks selectedCount={selectedCount} disabled={starting} onAsk={submit} />
    </div>
    <form onSubmit={event => { event.preventDefault(); submit(question); }} className="m-3 mt-0 flex shrink-0 items-end gap-2 rounded-xl border border-border bg-card p-1.5 focus-within:ring-2 focus-within:ring-ring">
      <textarea
        aria-label="Ask about your storage"
        value={question}
        rows={3}
        placeholder="What's using my space?"
        onChange={event => setQuestion(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); submit(question); } }}
        className="min-w-0 flex-1 resize-none bg-transparent px-1.5 py-1 text-ui text-foreground outline-none placeholder:text-muted-foreground"
      />
      <button type="submit" aria-label={starting ? "Starting" : "Ask"} disabled={!question.trim() || starting} className="grid size-7 shrink-0 place-items-center rounded-md bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-30">
        <ArrowUp size={14} aria-hidden="true" />
      </button>
    </form>
  </aside>;
}
