import type { ClipboardEvent } from "react";
import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUpRight, X } from "lucide-react";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { ChatModelControl } from "./ChatModelControl";
import { HarnessMark, harnessTintClass } from "./harnessMarks";
import { harnessLabel } from "../utils";
import { bridgeApi } from "../api";
import { mergeForestSnapshot } from "../forest";
import { startSerialPoll } from "../polling";
import { appendFileMention } from "../fileMentions";
import { activeTurnAction } from "../sessionInput";
import { type ComposerAttachment, imageFilesFromClipboard, isPasteTooLarge, mediaTypeOf, readAsDataUri } from "../pasteAttachments";
import { cn } from "@/lib/utils";
import type { AdapterDescriptor, AgentEvent, ApprovalDecision, Harness, Session, SessionForestSnapshot } from "../types";
import type { InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";

// An aside: a standalone chat the user delegated to another agent from inside
// a conversation, shown as a panel floating over that conversation instead of
// replacing it. The same shape as an orchestrator delegating to a worker, with
// the person in the orchestrator's seat: the aside gets the projected handoff
// brief of the chat it was asked from, answers in its own session, and stays a
// real chat in the sidebar after the panel closes. The panel is the delegation
// surface, not the session's home; reopening later is ordinary navigation.

export function AsideChat({ session, adapters, events, pendingMessages, working, modelSwitch = null, onSend, onChangeModel, onResolve, onAnswerQuestion = async () => undefined, onPromote, onClose }: {
  session: Session;
  /** The chat adapters, for the header model picker. */
  adapters: AdapterDescriptor[];
  /** The global live stream; the panel filters to its own session. */
  events: AgentEvent[];
  pendingMessages: string[];
  working: boolean;
  /** This aside's model switch in flight, for the same "Switching to …"
   *  narration the main conversation shows. */
  modelSwitch?: { harness: string; label: string } | null;
  onSend: (text: string, attachments?: ComposerAttachment[]) => Promise<void>;
  /** Pick which model the side chat runs on; applies on the next message.
   *  May reject — the panel wears the failure itself, because the main error
   *  banner sits behind the scrim where nobody is looking. */
  onChangeModel: (harness: Harness, model: string | null) => void | Promise<void>;
  onResolve: (eventId: number, decision: ApprovalDecision, optionId?: string) => Promise<InteractionResolutionResult | void> | void;
  onAnswerQuestion?: (eventId: number, action: QuestionAction, answers: Record<string, string[]>) => Promise<InteractionResolutionResult | void> | void;
  /** Make the aside the active session and close the panel. */
  onPromote: () => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [composerError, setComposerError] = useState<string>();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const forestKeyRef = useRef("");
  const ownEvents = events.filter(event => event.sessionId === session.id);

  // The durable side of the transcript: without it the handoff brief the aside
  // was created around is invisible, because the brief is a forest entry and
  // never a live frame. Digest-gated the same way the main conversation polls
  // its own forest: the live stream can grow every frame during a turn, and a
  // full snapshot refetch on every frame is what used to hang the panel. Only
  // the cheap digest is checked that often; the snapshot itself is only
  // refetched when the digest actually moves.
  useEffect(() => {
    forestKeyRef.current = "";
    setForest(undefined);
    let active = true;
    let pollsSinceFullFetch = 0;
    const refresh = async () => {
      const digest = await bridgeApi.sessionForestDigest(session.id).catch(() => undefined);
      const force = pollsSinceFullFetch >= 9 || digest === undefined;
      if (!active) return;
      if (!force && digest === forestKeyRef.current) {
        pollsSinceFullFetch += 1;
        return;
      }
      const value = await bridgeApi.sessionForest(session.id).catch(() => undefined);
      if (!active) return;
      pollsSinceFullFetch = 0;
      if (!value) return;
      forestKeyRef.current = digest ?? "";
      setForest(current => mergeForestSnapshot(current, value));
    };
    const stop = startSerialPoll(refresh, 3000);
    return () => { active = false; stop(); };
  }, [session.id]);

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
    const sentAttachments = attachments;
    if (!text && sentAttachments.length === 0) return;
    setDraft("");
    setAttachments([]);
    await onSend(text, sentAttachments);
  }

  // Mirrors the main composer's `handleComposerPaste`: clipboard images become
  // removable preview chips instead of inserted text; every other paste falls
  // through untouched. Sized before decode, same reasoning as the main chat —
  // a silent multi-second paste for a huge screenshot reads as broken.
  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const items = event.clipboardData?.items;
    if (!items) return;
    const files = imageFilesFromClipboard(items);
    if (files.length === 0) return;
    event.preventDefault();
    if (files.some(isPasteTooLarge)) {
      setComposerError("That image is too large to paste (over 8 MB).");
      return;
    }
    setComposerError(undefined);
    void Promise.all(files.map(async file => ({
      id: crypto.randomUUID(),
      mediaType: mediaTypeOf(file),
      dataUri: await readAsDataUri(file),
    })))
      .then(pasted => setAttachments(current => [...current, ...pasted]))
      .catch(error => setComposerError(error instanceof Error ? error.message : String(error)));
    inputRef.current?.focus();
  };

  // The `+` control, mirroring the main composer's `attachFile`: the system
  // file dialog inside the desktop shell, turning picks into `@path`
  // mentions. Outside Tauri there is no dialog and the aside has no mention
  // picker of its own, so it drops a bare `@` for the user to keep typing.
  async function attachFile() {
    if (!("__TAURI_INTERNALS__" in window)) {
      setDraft(current => (current.length === 0 || /\s$/.test(current) ? `${current}@` : `${current} @`));
      inputRef.current?.focus();
      return;
    }
    try {
      const picked = await open({ multiple: true, title: "Attach files" });
      if (picked == null) return;
      const paths = (Array.isArray(picked) ? picked : [picked]).filter((path): path is string => typeof path === "string");
      if (paths.length === 0) return;
      setDraft(current => paths.reduce(appendFileMention, current));
    } catch (e) {
      setComposerError(e instanceof Error ? e.message : String(e));
    } finally {
      inputRef.current?.focus();
    }
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
        <header className="flex min-h-[3.25rem] shrink-0 select-none items-center gap-2.5 border-b border-border px-4 py-1.5">
          <HarnessMark harness={session.harness} live={working} size={15}/>
          <div className="min-w-0 flex-1">
            <h2 className="m-0 truncate font-display text-[13px] font-semibold leading-tight text-foreground">{session.title || session.label}</h2>
            <div className="-ml-1 flex min-w-0 items-center gap-1">
              <ChatModelControl
                adapters={adapters}
                harness={session.harness}
                model={session.model ?? null}
                disabled={working || !!modelSwitch}
                disabledReason={working ? "Wait for the current response before switching models" : undefined}
                onChange={(harness, model) => {
                  setComposerError(undefined);
                  void Promise.resolve(onChangeModel(harness, model))
                    .catch(error => setComposerError(error instanceof Error ? error.message : String(error)));
                }}
                compact
                roleLabel="Aside"
                placement="down"
                maxWidthClassName="max-w-[220px]"
              />
              <span className={cn("shrink-0 text-[10px] leading-tight", harnessTintClass(session.harness))}>aside</span>
            </div>
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
            modelSwitch={modelSwitch}
            onResolve={onResolve}
            onAnswerQuestion={onAnswerQuestion}
          />
        </div>

        <footer className="shrink-0 border-t border-border p-3">
          {composerError && <p className="mb-2 px-1 text-[11px] text-destructive">{composerError}</p>}
          <ComposerPill
            layout="dock"
            className="mx-0 max-w-none px-0 pb-0 pt-0 sm:px-0 sm:pb-0"
            value={draft}
            onChange={setDraft}
            onSubmit={() => void send()}
            onPaste={handlePaste}
            attachments={attachments}
            onRemoveAttachment={id => setAttachments(current => current.filter(attachment => attachment.id !== id))}
            placeholder={`Ask ${harnessLabel(session.harness)}…`}
            working={working}
            activeAction={activeTurnAction(adapters.find(adapter => adapter.id === session.harness)?.capabilities)}
            onPlusClick={() => void attachFile()}
            inputRef={inputRef}
          />
        </footer>
      </div>
    </div>
  );
}
