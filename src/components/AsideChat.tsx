import { Dialog, DialogPopup } from "@/components/ui/dialog";
import type { ClipboardEvent, KeyboardEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUpRight, FileText, X } from "lucide-react";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { ChatModelControl } from "./ChatModelControl";
import { HarnessMark, harnessTintClass } from "./harnessMarks";
import { harnessLabel, slashCommandsForHarness, slashOwnershipBadge } from "../utils";
import { bridgeApi } from "../api";
import { mergeForestSnapshot } from "../forest";
import { startSerialPoll } from "../polling";
import { appendFileMention, applyFileMention as insertFileMention, fileMentionQuery } from "../fileMentions";
import { scheduleSuggestion } from "../suggestionTypeahead";
import { activeTurnAction } from "../sessionInput";
import { SIDE_CHAT_COMMANDS } from "../sideChat";
import { type ComposerAttachment, imageFilesFromClipboard, isPasteTooLarge, mediaTypeOf, readAsDataUri } from "../pasteAttachments";
import { cn } from "@/lib/utils";
import type { AdapterDescriptor, AgentEvent, ApprovalDecision, Harness, Session, SessionForestSnapshot, SlashCommand } from "../types";
import type { InteractionResolutionResult, QuestionAction, SuggestCompletionResult, SuggestionSettingsSnapshot } from "../protocol/generated/protocol";

// An aside: a standalone chat the user delegated to another agent from inside
// a conversation, shown as a panel floating over that conversation instead of
// replacing it. The same shape as an orchestrator delegating to a worker, with
// the person in the orchestrator's seat: the aside gets the projected handoff
// brief of the chat it was asked from, answers in its own session, and stays a
// real chat in the sidebar after the panel closes. The panel is the delegation
// surface, not the session's home; reopening later is ordinary navigation.

export function AsideChat({ session, adapters, events, pendingMessages, working, modelSwitch = null, lifecycle, initialDraft, workspaceFiles = [], slashCommands = [], onSend, onChangeModel, onChangeEffort, onResolve, onAnswerQuestion = async () => undefined, onRetryCompaction, onPromote, onClose }: {
  session: Session;
  /** The chat adapters, for the header model picker. */
  adapters: AdapterDescriptor[];
  /** The global live stream; the panel filters to its own session. */
  events: AgentEvent[];
  pendingMessages: string[];
  working: boolean;
  /** Workspace paths for the same `@` mention typeahead the main composer uses. */
  workspaceFiles?: string[];
  /** Slash commands already loaded by the host; the aside does not refetch them. */
  slashCommands?: SlashCommand[];
  /** This aside's model switch in flight, for the same "Switching to …"
   *  narration the main conversation shows. */
  modelSwitch?: { harness: string; label: string } | null;
  lifecycle?: { phase: string; handoffStatus?: string; fidelity?: string; error?: string };
  /** A first delivery that failed is handed back to this panel for retry. */
  initialDraft?: string;
  onSend: (text: string, attachments?: ComposerAttachment[]) => Promise<void>;
  /** Pick which model the side chat runs on; applies on the next message.
   *  May reject — the panel wears the failure itself, because the main error
   *  banner sits behind the scrim where nobody is looking. */
  onChangeEffort?: (effort: string) => void | Promise<void>;
  onChangeModel: (harness: Harness, model: string | null) => void | Promise<void>;
  onResolve: (eventId: number, decision: ApprovalDecision, optionId?: string) => Promise<InteractionResolutionResult | void> | void;
  onAnswerQuestion?: (eventId: number, action: QuestionAction, answers: Record<string, string[]>) => Promise<InteractionResolutionResult | void> | void;
  onRetryCompaction?: () => Promise<void>;
  /** Make the aside the active session and close the panel. */
  onPromote: () => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [composerError, setComposerError] = useState<string>();
  const [sending, setSending] = useState(false);
  const [changingEffort, setChangingEffort] = useState(false);
  const [queuedFollowUps, setQueuedFollowUps] = useState<{ text: string; attachments: ComposerAttachment[] }[]>([]);
  const queuedFollowUpsRef = useRef<{ text: string; attachments: ComposerAttachment[] }[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionDismissed, setMentionDismissed] = useState(false);
  const [suggestionSettings, setSuggestionSettings] = useState<SuggestionSettingsSnapshot>();
  const [draftSuggestion, setDraftSuggestion] = useState<SuggestCompletionResult>();
  const suggestionGeneration = useRef(0);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const slashListRef = useRef<HTMLDivElement>(null);
  const mentionListRef = useRef<HTMLDivElement>(null);
  const forestKeyRef = useRef("");
  const typeaheadOpenRef = useRef(false);
  const ownEvents = events.filter(event => event.sessionId === session.id);

  const slashQuery = /^\/([^\s]*)$/.exec(draft)?.[1];
  const slashMatches = useMemo(() => {
    if (slashQuery == null) return [];
    const query = slashQuery.toLowerCase();
    // A side chat cannot open a side chat of its own: the panel is a single
    // overlay and the aside is pinned to its harness, so Bridge's /btw and
    // /side are hidden here instead of offered and then refused on send.
    return slashCommandsForHarness(slashCommands, session.harness)
      .filter(command => !SIDE_CHAT_COMMANDS.includes(command.name.toLowerCase()))
      .filter(command => !query || command.name.toLowerCase().includes(query) || command.description.toLowerCase().includes(query))
      .sort((a, b) => {
        const aName = a.name.toLowerCase();
        const bName = b.name.toLowerCase();
        const aPrefix = query ? Number(aName.startsWith(query)) : 0;
        const bPrefix = query ? Number(bName.startsWith(query)) : 0;
        if (aPrefix !== bPrefix) return bPrefix - aPrefix;
        return aName.localeCompare(bName);
      });
  }, [slashQuery, slashCommands, session.harness]);
  const slashOpen = slashQuery != null && slashMatches.length > 0 && !slashDismissed;

  const mentionQuery = fileMentionQuery(draft);
  const workspaceFileOptions = useMemo(() => workspaceFiles.map(path => {
    const lowerPath = path.toLowerCase();
    return { path, lowerPath, lowerBase: lowerPath.split("/").pop() ?? lowerPath };
  }), [workspaceFiles]);
  const fileMatches = useMemo(() => {
    if (mentionQuery == null) return [];
    const query = mentionQuery.toLowerCase();
    return workspaceFileOptions
      .filter(file => !query || file.lowerPath.includes(query))
      .sort((a, b) => {
        const aPrefix = query ? Number(a.lowerBase.startsWith(query) || a.lowerPath.startsWith(query)) : 0;
        const bPrefix = query ? Number(b.lowerBase.startsWith(query) || b.lowerPath.startsWith(query)) : 0;
        if (aPrefix !== bPrefix) return bPrefix - aPrefix;
        return a.path.length - b.path.length || a.path.localeCompare(b.path);
      })
      .slice(0, 50)
      .map(file => file.path);
  }, [mentionQuery, workspaceFileOptions]);
  const mentionOpen = mentionQuery != null && fileMatches.length > 0 && !mentionDismissed;

  useEffect(() => {
    if (initialDraft) setDraft(current => current || initialDraft);
  }, [initialDraft]);

  useEffect(() => { void bridgeApi.getSuggestionSettings().then(setSuggestionSettings).catch(() => undefined); }, []);

  useEffect(() => scheduleSuggestion({
    text: draft,
    enabled: !!suggestionSettings?.settings.enabled,
    request: bridgeApi.suggestCompletion,
    onResult: setDraftSuggestion,
    generation: suggestionGeneration,
  }), [draft, suggestionSettings?.settings.enabled, suggestionSettings?.settings.provider, suggestionSettings?.settings.model]);

  useEffect(() => {
    if (!mentionOpen) return;
    setMentionIndex(index => Math.min(index, Math.max(0, fileMatches.length - 1)));
  }, [mentionOpen, fileMatches.length]);

  useEffect(() => {
    if (!mentionOpen) return;
    const root = mentionListRef.current;
    if (!root) return;
    root.querySelector<HTMLElement>(`[data-mention-index="${mentionIndex}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [mentionOpen, mentionIndex]);

  useEffect(() => {
    if (!slashOpen) return;
    const root = slashListRef.current;
    if (!root) return;
    root.querySelector<HTMLElement>(`[data-slash-index="${slashIndex}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [slashOpen, slashIndex]);

  typeaheadOpenRef.current = mentionOpen || slashOpen;

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
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (typeaheadOpenRef.current) return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    inputRef.current?.focus();
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  async function send() {
    const text = draft.trim();
    const sentAttachments = attachments;
    if (!text && sentAttachments.length === 0) return;
    if (sending) {
      queuedFollowUpsRef.current = [...queuedFollowUpsRef.current, { text, attachments: sentAttachments }];
      setQueuedFollowUps(queuedFollowUpsRef.current);
      setDraft("");
      setAttachments([]);
      return;
    }
    setSending(true);
    setComposerError(undefined);
    setDraft("");
    setAttachments([]);
    try {
      await onSend(text, sentAttachments);
      const queued = queuedFollowUpsRef.current;
      queuedFollowUpsRef.current = [];
      setQueuedFollowUps([]);
      for (const followUp of queued) await onSend(followUp.text, followUp.attachments);
    } catch (error) {
      setDraft(text);
      setAttachments(sentAttachments);
      setComposerError(error instanceof Error ? error.message : String(error));
    } finally {
      setSending(false);
      inputRef.current?.focus();
    }
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
    attachFiles(files);
  };
  const attachFiles = (files: Array<{ type: string; size?: number }>) => {
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
  function applySlash(command: SlashCommand) {
    setDraft(`/${command.name} `);
    setSlashIndex(0);
    setSlashDismissed(true);
  }

  function applyMention(path: string) {
    setDraft(current => insertFileMention(current, path));
    setMentionIndex(0);
    setMentionDismissed(true);
  }

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.nativeEvent.isComposing) return;
    if (mentionOpen) {
      if (event.key === "ArrowDown") { event.preventDefault(); setMentionIndex(index => Math.min(index + 1, fileMatches.length - 1)); return; }
      if (event.key === "ArrowUp") { event.preventDefault(); setMentionIndex(index => Math.max(index - 1, 0)); return; }
      if (event.key === "Escape") { event.preventDefault(); setMentionDismissed(true); return; }
      if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") { event.preventDefault(); applyMention(fileMatches[Math.min(mentionIndex, fileMatches.length - 1)]); return; }
    }
    if (slashOpen) {
      if (event.key === "ArrowDown") { event.preventDefault(); setSlashIndex(index => Math.min(index + 1, slashMatches.length - 1)); return; }
      if (event.key === "ArrowUp") { event.preventDefault(); setSlashIndex(index => Math.max(index - 1, 0)); return; }
      if (event.key === "Escape") { event.preventDefault(); setSlashDismissed(true); return; }
      if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") { event.preventDefault(); applySlash(slashMatches[Math.min(slashIndex, slashMatches.length - 1)]); return; }
    }
  }

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
    <Dialog open onOpenChange={next => { if (!next && !typeaheadOpenRef.current) onClose(); }}>
      <DialogPopup showCloseButton={false} initialFocus={inputRef} aria-label={`Aside with ${harnessLabel(session.harness)}`} className="h-[min(720px,84dvh)] max-w-2xl">
        <header className="flex min-h-[3.25rem] shrink-0 select-none items-center gap-2.5 border-b border-border px-4 py-1.5">
          <HarnessMark harness={session.harness} live={working} size={15}/>
          <div className="min-w-0 flex-1">
            <h2 className="m-0 truncate font-display text-[13px] font-semibold leading-tight text-foreground">{session.title || session.label}</h2>
            <div className="-ml-1 flex min-w-0 items-center gap-1">
              <ChatModelControl
                adapters={adapters}
                harness={session.harness}
                model={session.model ?? null}
                disabled={working || !!modelSwitch || changingEffort}
                disabledReason={working ? "Wait for the current response before switching models" : undefined}
                onChange={(harness, model) => {
                  setComposerError(undefined);
                  void Promise.resolve(onChangeModel(harness, model))
                    .catch(error => setComposerError(error instanceof Error ? error.message : String(error)));
                }}
                effort={session.effort}
                onEffortChange={onChangeEffort ? effort => {
                  setComposerError(undefined);
                  setChangingEffort(true);
                  void Promise.resolve().then(() => onChangeEffort(effort))
                    .catch(error => setComposerError(error instanceof Error ? error.message : String(error)))
                    .finally(() => setChangingEffort(false));
                } : undefined}
                compact
                roleLabel="Aside"
                placement="down"
                maxWidthClassName="max-w-[220px]"
              />
              <span className={cn("shrink-0 text-[11px] leading-tight", harnessTintClass(session.harness))}>aside</span>
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
            onRetryCompaction={onRetryCompaction}
          />
        </div>

        <footer className="shrink-0 border-t border-border p-3">
          {lifecycle?.handoffStatus && <p className="mb-2 px-1 text-[11px] text-muted-foreground">
            {lifecycle.handoffStatus === "forked"
              ? "Native fork of the conversation it was asked from"
              : lifecycle.handoffStatus === "carried" ? "Context carried" : "No prior context available"}
            {lifecycle.fidelity === "projected_at_boundary" ? " · projected at the handoff boundary" : ""}
          </p>}
          {lifecycle?.phase === "switching" && <p className="mb-2 px-1 text-[11px] text-muted-foreground">Preparing a handoff and switching models. This can take up to 30 seconds…</p>}
          {sending && <p className="mb-2 px-1 text-[11px] text-muted-foreground">Sending…</p>}
          {queuedFollowUps.length > 0 && <p className="mb-2 px-1 text-[11px] text-muted-foreground" role="status">{queuedFollowUps.length} follow-up{queuedFollowUps.length === 1 ? "" : "s"} queued — sent when this send finishes</p>}
          {lifecycle?.error && !composerError && <p className="mb-2 px-1 text-[11px] text-destructive">{lifecycle.error} You can retry below.</p>}
          {composerError && <p className="mb-2 px-1 text-[11px] text-destructive">{composerError}</p>}
          <div className="relative">
            {mentionOpen && <div id="aside-file-mention-listbox" role="listbox" className="u-glass-popover absolute inset-x-0 bottom-full z-20 mb-2 flex max-h-[min(320px,45vh)] flex-col overflow-hidden rounded-2xl">
              <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5 text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
                <span>Reference a file</span>
                <span className="normal-case tracking-normal text-muted-foreground">{fileMatches.length}</span>
              </div>
              <div ref={mentionListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
                {fileMatches.map((file, index) => {
                  const dir = file.includes("/") ? file.slice(0, file.lastIndexOf("/") + 1) : "";
                  const base = file.slice(dir.length);
                  return <button id={`aside-file-mention-option-${index}`} role="option" aria-selected={index === mentionIndex} key={file} type="button" data-mention-index={index} onMouseEnter={() => setMentionIndex(index)} onMouseDown={e => { e.preventDefault(); applyMention(file); }} className={`flex min-h-11 w-full items-center gap-2 px-3 py-2 text-left transition-colors ${index === mentionIndex ? "bg-accent" : "hover:bg-accent"}`}>
                    <FileText size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[12px]"><span className="text-muted-foreground">{dir}</span><span className="text-foreground">{base}</span></span>
                  </button>;
                })}
              </div>
            </div>}
            {slashOpen && <div id="aside-slash-listbox" role="listbox" className="u-glass-popover absolute inset-x-0 bottom-full z-20 mb-2 flex max-h-[min(320px,45vh)] flex-col overflow-hidden rounded-2xl">
              <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5 text-[11px] uppercase tracking-[0.12em] text-muted-foreground">
                <span>Commands & skills</span>
                <span className="normal-case tracking-normal text-muted-foreground">{slashMatches.length}</span>
              </div>
              <div ref={slashListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
                {slashMatches.map((command, index) => <button id={`aside-slash-option-${index}`} key={`${command.harness}:${command.kind}:${command.name}`} type="button" role="option" aria-selected={index === slashIndex} data-slash-index={index} onMouseEnter={() => setSlashIndex(index)} onMouseDown={e => { e.preventDefault(); applySlash(command); }} className={`flex w-full items-center gap-2 px-3 py-2 text-left transition-colors ${index === slashIndex ? "bg-accent" : "hover:bg-accent"}`}>
                  <span className="whitespace-nowrap font-mono text-[12px] text-foreground">/{command.name}</span>
                  <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-muted-foreground">{command.description}</span>
                  <span className="shrink-0 rounded border border-border px-1 py-[1px] text-[11px] uppercase tracking-[0.06em] text-muted-foreground">{slashOwnershipBadge(command.harness)}</span>
                </button>)}
              </div>
            </div>}
            <ComposerPill
              layout="dock"
              className="mx-0 max-w-none px-0 pb-0 pt-0 sm:px-0 sm:pb-0"
              value={draft}
              onChange={value => { setDraft(value); setSlashDismissed(false); setSlashIndex(0); setMentionDismissed(false); setMentionIndex(0); }}
              onSubmit={() => void send()}
              onKeyDown={onComposerKeyDown}
              onPaste={handlePaste}
              onAttachFiles={attachFiles}
              attachments={attachments}
              onRemoveAttachment={id => setAttachments(current => current.filter(attachment => attachment.id !== id))}
              autocomplete={mentionOpen ? {
                controls: "aside-file-mention-listbox",
                activeDescendant: `aside-file-mention-option-${mentionIndex}`,
              } : slashOpen ? {
                controls: "aside-slash-listbox",
                activeDescendant: `aside-slash-option-${slashIndex}`,
              } : undefined}
              suggestion={draftSuggestion?.suggestion}
              onAcceptSuggestion={() => {
                if (!draftSuggestion?.suggestion) return;
                setDraft(current => `${current}${draftSuggestion.suggestion}`);
                setDraftSuggestion(undefined);
              }}
              placeholder={`Ask ${harnessLabel(session.harness)}…`}
              working={working}
              activeAction={activeTurnAction(adapters.find(adapter => adapter.id === session.harness)?.capabilities)}
              onPlusClick={() => void attachFile()}
              inputRef={inputRef}
            />
          </div>
        </footer>
      </DialogPopup>
    </Dialog>
  );
}
