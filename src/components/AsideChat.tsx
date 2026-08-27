import { useEffect, useRef, useState } from "react";
import { ArrowUp, ArrowUpRight, X } from "lucide-react";
import { AgentConversation } from "./AgentConversation";
import { HarnessMark, harnessTintClass } from "./harnessMarks";
import { harnessLabel, modelLabel } from "../utils";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import type { AgentEvent, ApprovalDecision, Session, SessionForestSnapshot } from "../types";

// An aside: a standalone chat the user delegated to another agent from inside
// a conversation, shown as a panel floating over that conversation instead of
// replacing it. The same shape as an orchestrator delegating to a worker, with
// the person in the orchestrator's seat: the aside gets the projected handoff
// brief of the chat it was asked from, answers in its own session, and stays a
// real chat in the sidebar after the panel closes. The panel is the delegation
// surface, not the session's home; reopening later is ordinary navigation.

export function AsideChat({ session, events, pendingMessages, working, onSend, onResolve, onPromote, onClose }: {
  session: Session;
  /** The global live stream; the panel filters to its own session. */
  events: AgentEvent[];
  pendingMessages: string[];
  working: boolean;
  onSend: (text: string) => Promise<void>;
  onResolve: (eventId: number, decision: ApprovalDecision) => void;
  /** Make the aside the active session and close the panel. */
  onPromote: () => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const ownEvents = events.filter(event => event.sessionId === session.id);

  // The durable side of the transcript: without it the handoff brief the aside
  // was created around is invisible, because the brief is a forest entry and
  // never a live frame. Refetched as the live stream grows, so durable rows
  // (checkpoints, briefs) keep up with the conversation; AgentConversation
  // dedupes the overlap.
  useEffect(() => {
    let live = true;
    bridgeApi.sessionForest(session.id)
      .then(snapshot => { if (live) setForest(snapshot); })
      .catch(() => undefined);
    return () => { live = false; };
  }, [session.id, ownEvents.length]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKeyDown);
    inputRef.current?.focus();
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  async function send() {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    await onSend(text);
  }

  return (
    <div
      className="absolute inset-0 z-40 flex items-center justify-center bg-scrim p-4 backdrop-blur-[2px] sm:p-8"
      role="presentation"
      onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Aside with ${harnessLabel(session.harness)}`}
        className="u-glass-popover animate-page-enter flex h-full max-h-[720px] w-full max-w-[640px] min-w-0 flex-col overflow-hidden rounded-2xl"
      >
        <header className="flex h-11 shrink-0 select-none items-center gap-2.5 border-b border-border px-4">
          <HarnessMark harness={session.harness} live={working} size={15}/>
          <div className="min-w-0 flex-1">
            <h2 className="m-0 truncate font-display text-[13px] font-semibold leading-tight text-foreground">{session.title || session.label}</h2>
            <p className={cn("m-0 truncate text-[10px] leading-tight", harnessTintClass(session.harness))}>
              {harnessLabel(session.harness)}{session.model ? ` · ${modelLabel(session.model)}` : ""} · aside
            </p>
          </div>
          <button
            type="button"
            onClick={onPromote}
            className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            title="Continue this aside as a full chat"
          >
            Open as chat
            <ArrowUpRight size={12} aria-hidden="true"/>
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close aside"
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <X size={14} aria-hidden="true"/>
          </button>
        </header>

        <div className="relative min-h-0 flex-1">
          <AgentConversation
            session={session}
            events={ownEvents}
            forestEntries={forest?.entries}
            activeLeafId={forest?.head?.activeEntryId ?? null}
            working={working}
            pendingMessages={pendingMessages}
            onResolve={onResolve}
          />
        </div>

        <footer className="shrink-0 border-t border-border p-3">
          <div className="flex items-end gap-2 rounded-2xl bg-accent/60 px-3 py-2">
            <textarea
              ref={inputRef}
              value={draft}
              rows={1}
              placeholder={`Ask ${harnessLabel(session.harness)}…`}
              onChange={event => setDraft(event.target.value)}
              onKeyDown={event => {
                if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void send(); }
              }}
              className="max-h-28 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-6 text-foreground outline-none placeholder:text-muted-foreground/60"
            />
            <button
              type="button"
              onClick={() => void send()}
              disabled={!draft.trim()}
              aria-label="Send to the aside"
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground transition-opacity disabled:opacity-35"
            >
              <ArrowUp size={14} aria-hidden="true"/>
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
