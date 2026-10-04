// "In conversations": the sidebar's cross-chat search cards.
//
// The title filter above answers "which chat is called that"; these cards
// answer "which chat talked about that". Each card is one click target that
// opens the chat. The footer says what Enter will do, so the deep stage is
// discoverable without knowing it exists.

import { LoaderCircle, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ChatSearchHit, SearchChatsResult } from "../protocol/generated/protocol";
import { HarnessMark } from "./harnessMarks";
import { chatListTime } from "./sidebarChats";

function hitTime(hit: ChatSearchHit, now: number): string | null {
  const at = Date.parse(hit.lastActiveAt);
  return chatListTime(Number.isFinite(at) ? at : null, now);
}

function footer(result: SearchChatsResult | undefined, deepRunning: boolean, loading: boolean): string | null {
  if (deepRunning) return "Searching deeper…";
  if (loading || !result) return null;
  if (result.stage === "index_fallback") return result.detail ?? "Showing the index's matches.";
  if (result.stage === "model") return result.hits.length ? "Enter opens the first match." : null;
  if (!result.hits.length) {
    return result.deepAvailable ? "No direct matches. Press Enter to search deeper." : result.detail ?? null;
  }
  if (result.confident || !result.deepAvailable) {
    return result.detail && !result.confident ? `${result.detail} Enter opens the first match.` : "Enter opens the first match.";
  }
  return "Not sure which one? Press Enter to search deeper.";
}

export function ChatSearchResults({
  result,
  loading,
  deepRunning,
  error,
  activeSessionId,
  now,
  onOpen,
}: {
  result?: SearchChatsResult;
  loading: boolean;
  deepRunning: boolean;
  error?: string;
  activeSessionId?: string;
  now: number;
  onOpen: (sessionId: string) => void;
}) {
  const hits = result?.hits ?? [];
  const note = error ?? footer(result, deepRunning, loading);
  return (
    <section aria-label="In conversations" aria-busy={loading || deepRunning} className="mb-3 flex flex-col gap-0.5">
      <div className="flex h-7 items-center gap-1.5 px-2">
        <Search size={13} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="text-[12px] font-medium tracking-[-0.004em] text-muted-foreground">In conversations</span>
        {(loading || deepRunning) && <LoaderCircle size={12} strokeWidth={1.7} className="ml-auto animate-spin text-muted-foreground" aria-hidden="true" />}
      </div>
      <ul className={cn("flex flex-col gap-0.5 transition-opacity", deepRunning && "opacity-60")}>
        {hits.map(hit => {
          const time = hitTime(hit, now);
          const active = hit.sessionId === activeSessionId;
          return (
            <li key={hit.sessionId}>
              <button
                type="button"
                onClick={() => onOpen(hit.sessionId)}
                title={hit.title}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex w-full min-w-0 flex-col gap-0.5 rounded-[7px] px-2 py-1.5 text-left transition-colors active:scale-[0.99]",
                  active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
                )}
              >
                <span className="flex min-w-0 items-center gap-1.5">
                  <HarnessMark harness={hit.harness} size={12} className="shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate text-[13px] leading-4 tracking-[-0.008em] text-foreground">{hit.title}</span>
                  {time && <span className="shrink-0 text-[11px] tabular-nums text-faint">{time}</span>}
                </span>
                {hit.snippet && <span className="line-clamp-2 text-[11px] leading-3.5 text-muted-foreground">{hit.snippet}</span>}
                <span className="flex min-w-0 items-center gap-1.5 text-[11px] leading-3.5 text-faint">
                  <span className="truncate">{hit.why}</span>
                  {(hit.archived || hit.ended) && <span className="shrink-0">· {hit.archived ? "archived" : "ended"}</span>}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      {note && <p role="status" className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">{note}</p>}
    </section>
  );
}
