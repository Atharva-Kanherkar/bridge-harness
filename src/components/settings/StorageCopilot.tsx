// The Storage page's copilot rail: a chat docked beside the listing. Each
// storage chat is a direct chat created with the `storage` purpose, so its
// brief is a system prompt of its own (`storage_agent.rs`). A visit opens on a
// blank rail; earlier storage chats are listed under Recent and stay reachable
// from the sidebar like any other chat.

import { useState, type ReactNode } from "react";
import { ArrowUp, Boxes, ChevronRight, Container, FolderSearch, HardDrive, ListChecks, MessageSquare, Sparkles } from "lucide-react";
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
  /** The page opened: the host may clear the rail back to a blank chat. */
  onVisit?: () => void;
  /** The storage chat is taking a turn. When one ends the page measures
   *  again, since the agent may have cleaned something up itself. */
  working?: boolean;
}

/** An earlier storage chat the rail can reopen. */
export interface StorageRecentChat { id: string; title: string; at: string }

export function StorageCopilot({ chat, selectedCount, starting = false, recent = [], onOpenRecent, onAsk }: {
  /** The docked chat, once a storage chat is open. */
  chat?: ReactNode;
  selectedCount: number;
  starting?: boolean;
  recent?: readonly StorageRecentChat[];
  onOpenRecent?: (id: string) => void;
  onAsk: (question: string) => void;
}) {
  if (chat) return <div className={RAIL}>{chat}</div>;
  return <StorageCopilotIntro selectedCount={selectedCount} starting={starting} recent={recent} onOpenRecent={onOpenRecent} onAsk={onAsk} />;
}

/** The rail fills the window beside the page (title bar, sticky offset, and
 *  bottom gap taken out) and stays put while the listing scrolls. */
const RAIL = "flex h-[38rem] flex-col lg:sticky lg:top-6 lg:h-[calc(100dvh-6rem)] lg:self-start";

const selectedAsk = (count: number) => `Tell me what the ${count} item${count === 1 ? "" : "s"} I selected are, and whether I can delete them.`;

const when = (at: string) => {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return "";
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000);
  if (days < 1) return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (days < 7) return date.toLocaleDateString([], { weekday: "short" });
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
};

/** One-click requests, as a grouped list: an icon, the request, a chevron. */
function QuickAsks({ selectedCount, disabled, onAsk }: { selectedCount: number; disabled: boolean; onAsk: (prompt: string) => void }) {
  const asks = selectedCount > 0
    ? [{ label: `Explain the ${selectedCount} selected`, prompt: selectedAsk(selectedCount), icon: ListChecks, series: "bg-ctx-6" }, ...STORAGE_QUICK_ASKS]
    : STORAGE_QUICK_ASKS;
  return <ul aria-label="Quick asks" className="divide-y divide-border/60 overflow-hidden rounded-xl bg-muted/40 ring-1 ring-inset ring-border/60">
    {asks.map(ask => {
      const Icon = ask.icon;
      return <li key={ask.label}>
        <button type="button" disabled={disabled} onClick={() => onAsk(ask.prompt)} className="group flex w-full items-center gap-2.5 px-2.5 py-2 text-left text-caption text-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent disabled:opacity-40">
          <span aria-hidden="true" className={cn("grid size-6 shrink-0 place-items-center rounded-md text-background", ask.series)}><Icon size={13} /></span>
          <span className="min-w-0 flex-1 truncate">{ask.label}</span>
          <ChevronRight size={13} aria-hidden="true" className="shrink-0 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5" />
        </button>
      </li>;
    })}
  </ul>;
}

function StorageCopilotIntro({ selectedCount, starting, recent, onOpenRecent, onAsk }: { selectedCount: number; starting: boolean; recent: readonly StorageRecentChat[]; onOpenRecent?: (id: string) => void; onAsk: (question: string) => void }) {
  const [question, setQuestion] = useState("");
  const submit = (text: string) => {
    if (!text.trim() || starting) return;
    onAsk(text);
    setQuestion("");
  };
  // Shaped like the chat it becomes: what it can do on top, the starting
  // points in the middle, the composer pinned to the bottom.
  return <aside aria-label="Ask Bridge" className={cn(RAIL, "overflow-hidden rounded-2xl border border-border bg-card shadow-sm")}>
    <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4 pt-6">
      <div className="flex flex-col items-center text-center">
        <span aria-hidden="true" className="grid size-10 place-items-center rounded-xl bg-gradient-to-b from-foreground/90 to-foreground/70 text-background shadow-sm"><HardDrive size={18} /></span>
        <h3 className="mt-3 font-display text-ui font-semibold text-foreground">Storage assistant</h3>
        <p className="mt-1 max-w-[18rem] text-caption leading-relaxed text-muted-foreground">
          Sees what this page measured, digs deeper with read-only commands, and proposes cleanups you approve. Nothing moves without you.
        </p>
      </div>
      <p className="mb-1.5 mt-6 px-1 text-[11px] font-medium text-muted-foreground">Suggestions</p>
      <QuickAsks selectedCount={selectedCount} disabled={starting} onAsk={submit} />
      {recent.length > 0 && onOpenRecent && <>
        <p className="mb-1.5 mt-5 px-1 text-[11px] font-medium text-muted-foreground">Recent</p>
        <ul aria-label="Recent storage chats" className="divide-y divide-border/60 overflow-hidden rounded-xl bg-muted/40 ring-1 ring-inset ring-border/60">
          {recent.map(item => <li key={item.id}>
            <button type="button" onClick={() => onOpenRecent(item.id)} className="flex w-full items-center gap-2.5 px-2.5 py-2 text-left text-caption outline-none transition-colors hover:bg-accent focus-visible:bg-accent">
              <MessageSquare size={13} aria-hidden="true" className="shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-foreground">{item.title}</span>
              <span className="shrink-0 tabular-nums text-muted-foreground">{when(item.at)}</span>
            </button>
          </li>)}
        </ul>
      </>}
    </div>
    <form onSubmit={event => { event.preventDefault(); submit(question); }} className="m-3 mt-0 flex shrink-0 items-end gap-2 rounded-xl border border-border bg-background p-1.5 transition-shadow focus-within:border-ring/60 focus-within:ring-3 focus-within:ring-ring/20">
      <textarea
        aria-label="Ask about your storage"
        value={question}
        rows={2}
        placeholder={starting ? "Starting…" : "Ask about your storage…"}
        onChange={event => setQuestion(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); submit(question); } }}
        className="min-w-0 flex-1 resize-none bg-transparent px-1.5 py-1 text-ui text-foreground outline-none placeholder:text-muted-foreground"
      />
      <button type="submit" aria-label={starting ? "Starting" : "Ask"} disabled={!question.trim() || starting} className="grid size-7 shrink-0 place-items-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-30">
        <ArrowUp size={14} aria-hidden="true" />
      </button>
    </form>
  </aside>;
}
