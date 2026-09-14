import type { ClipboardEvent, KeyboardEvent, MutableRefObject, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Paperclip, Plus, Square, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ComposerAttachment } from "@/pasteAttachments";

export type ComposerPillProps = {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  /// Intercepts the paste before the textarea's default text insertion. The
  /// handler decides whether the paste stays text or becomes attachments —
  /// calling `preventDefault` there is what stops the default insertion.
  onPaste?: (event: ClipboardEvent<HTMLTextAreaElement>) => void;
  /// Present when this surface accepts image attachments; renders the preview
  /// chips above the input and lets Enter send with no text at all.
  attachments?: ComposerAttachment[];
  onAttachFiles?: (files: File[]) => void;
  onRemoveAttachment?: (id: string) => void;
  placeholder?: string;
  disabled?: boolean;
  working?: boolean;
  /// What submitting does while the agent is working. `steer` means the provider
  /// takes the words into the turn in flight; `queue` means they are held and
  /// delivered at its next phase boundary. Omitted keeps the composer read-only
  /// during a turn, for surfaces that genuinely cannot be steered.
  activeAction?: "steer" | "queue";
  onStop?: () => void;
  /// Immediate feedback after Stop until the turn actually clears.
  stopping?: boolean;
  onPlusClick?: () => void;
  /// What the `+` control does on this surface, as the user reads it. A control
  /// whose label and behaviour disagree is worse than no control, so the label
  /// travels with the handler rather than being hardcoded here.
  plusLabel?: string;
  /// Set to explain why `+` is unavailable. Present means disabled, and the
  /// reason becomes the tooltip — an unexplained dead control is the thing this
  /// avoids.
  plusUnavailableReason?: string;
  /// Lets the owner put the caret back in the composer after an action of its
  /// own — opening the file picker from `+` is useless if the user then has to
  /// click into the box to filter it.
  inputRef?: MutableRefObject<HTMLTextAreaElement | null>;
  /// Optional control immediately after the attachment button.
  leading?: ReactNode;
  trailing?: ReactNode;
  /// The model chip (ChatModelControl) that leads the controls row, below the
  /// input. Restyle target 2a: the chip sits at the composer's leading edge with
  /// the access control beside it, rather than riding the far-right `trailing`
  /// slot. Optional so surfaces that keep the model picker on the right still work.
  modelControl?: ReactNode;
  /// The access / permission control ("Full access ▾"), shown after `modelControl`
  /// behind a hairline divider in the controls row.
  accessControl?: ReactNode;
  /// The context strip, rendered as an attached footer inside the composer frame:
  /// input → controls row → footer. When present it gets a top hairline and the
  /// recessed `bg-background` surface, so the frame visually contains it.
  footer?: ReactNode;
  /// Which glyph the attach/plus button wears. Chat surfaces attach files, so they
  /// pass "paperclip"; a surface whose `+` starts a new thing keeps the default plus.
  plusIcon?: "plus" | "paperclip";
  className?: string;
  layout?: "hero" | "dock";
  autocomplete?: { controls: string; activeDescendant?: string };
  /// The inline typeahead's continuation of `value`, rendered as ghost text
  /// right after it. Only ever shown while the caret sits at the end of the
  /// draft — a suggestion for text the user has since moved away from would
  /// be misleading, not helpful.
  suggestion?: string;
  /// Accept `suggestion` — appends it to `value`. Bound to Tab, and only when
  /// a suggestion is showing and the caret is still at the end of the draft.
  onAcceptSuggestion?: () => void;
};

const ACTIVE_ACTION_LABEL = { steer: "Steer", queue: "Queue" } as const;

export function ComposerPill({
  value,
  onChange,
  onSubmit,
  onKeyDown,
  onPaste,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  placeholder = "Ask Bridge…",
  disabled,
  working,
  activeAction,
  onStop,
  stopping = false,
  onPlusClick,
  plusLabel = "Attach a file",
  plusUnavailableReason,
  inputRef,
  leading,
  trailing,
  modelControl,
  accessControl,
  footer,
  plusIcon = "plus",
  className,
  layout = "dock",
  autocomplete,
  suggestion,
  onAcceptSuggestion,
}: ComposerPillProps) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  // Ghost text only makes sense continuing from where typing left off. Selection
  // changes do not re-render on their own, so a click into the middle of the
  // draft would leave the overlay painted; `caretEpoch` exists only to force a
  // render when the caret moves. Tab itself reads the caret live, so it cannot
  // accept a suggestion the overlay has not yet had a chance to hide.
  const caretAtEnd = () => {
    const node = textareaRef.current;
    return !!node && node.selectionStart === value.length && node.selectionEnd === value.length;
  };
  const [, setCaretEpoch] = useState(0);
  const showSuggestion = !!suggestion && caretAtEnd();
  const noteCaret = () => setCaretEpoch(n => n + 1);
  const syncOverlayScroll = () => {
    const overlay = overlayRef.current;
    const textarea = textareaRef.current;
    if (overlay && textarea) overlay.scrollTop = textarea.scrollTop;
  };
  const attachmentInput = useRef<HTMLInputElement>(null);
  const isHero = layout === "hero";
  // A working agent is exactly when supervision is worth the most, so a turn in
  // flight no longer locks the composer. Where an active turn cannot take input
  // at all (`activeAction` omitted) the old behaviour stands.
  const steerable = !!working && !!activeAction;
  const locked = !!disabled || (!!working && !activeAction);
  const hasAttachments = !!attachments && attachments.length > 0;
  // An image is a message on its own: a send with no text must stay possible.
  const canSend = !locked && (value.trim().length > 0 || hasAttachments);
  const submitLabel = steerable ? ACTIVE_ACTION_LABEL[activeAction] : "Send";

  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(textarea.scrollHeight, isHero ? 180 : 140)}px`;
  }, [value, isHero]);

  return (
    <div className={cn("w-full", isHero ? "mx-auto max-w-3xl" : "mx-auto max-w-conversation-frame px-4 pb-3 pt-2 sm:px-8 sm:pb-4", className)}>
      {/* The pill's box also anchors any floating composer controls. */}
      <div data-composer-frame className="relative">
        <form
          className={cn(
            "relative flex flex-col rounded-xl",
            // Resting surface: one ladder step above the canvas, no blur. The
            // hairline is the composer/input tone, a touch above the plain border.
            "border border-border-card bg-card shadow-control",
            "transition-colors duration-200",
            "focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/20",
          )}
          onSubmit={event => {
            event.preventDefault();
            if (canSend) onSubmit();
          }}
        >
          {/* Keep workspace metadata outside the writing surface. */}
          <div className={cn("flex flex-col gap-1", isHero ? "px-4 py-3.5 sm:px-5 sm:py-4" : "px-3 py-2")}>
          {hasAttachments && (
            <div className="flex flex-wrap items-center gap-2 px-1 pt-0.5">
              {attachments!.map(attachment => (
                <div
                  key={attachment.id}
                  className="group relative h-14 w-14 shrink-0 overflow-hidden rounded-lg border border-border bg-accent"
                >
                  <img
                    src={attachment.dataUri}
                    alt={`Attached image, ${attachment.mediaType}`}
                    className="h-full w-full object-cover"
                  />
                  {onRemoveAttachment && (
                    <button
                      type="button"
                      onClick={() => onRemoveAttachment(attachment.id)}
                      className="absolute right-0.5 top-0.5 grid h-6 w-6 place-items-center rounded-full bg-background/80 text-foreground opacity-90 transition-opacity hover:opacity-100"
                      aria-label="Remove attached image"
                    >
                      <X className="h-3 w-3" strokeWidth={2} aria-hidden="true" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
          <div className="relative">
            {/* The mirror overlay: `value` rendered invisibly so it occupies the
                same box the textarea's own text does, followed by the visible
                ghost text — which then only ever shows past where the real text
                ends. Sizing must track the textarea exactly, or the seam shows. */}
            {showSuggestion && (
              <div
                ref={overlayRef}
                aria-hidden="true"
                className={cn(
                  "pointer-events-none absolute inset-0 max-h-44 min-h-[28px] w-full overflow-hidden whitespace-pre-wrap break-words text-[15px] leading-relaxed tracking-[-0.006em]",
                  isHero ? "min-h-16 px-1 py-1" : "px-1 py-0.5",
                )}
              >
                <span className="invisible">{value}</span>
                <span className="text-muted-foreground/50">{suggestion}</span>
              </div>
            )}
            <textarea
              ref={node => {
                textareaRef.current = node;
                if (inputRef) inputRef.current = node;
              }}
              value={value}
              aria-label="Message Bridge"
              rows={1}
              placeholder={placeholder}
              disabled={locked}
              role={autocomplete ? "combobox" : undefined}
              aria-autocomplete={autocomplete ? "list" : undefined}
              aria-expanded={autocomplete ? true : undefined}
              aria-controls={autocomplete?.controls}
              aria-activedescendant={autocomplete?.activeDescendant}
              onChange={event => onChange(event.target.value)}
              onSelect={noteCaret}
              onClick={noteCaret}
              onKeyUp={noteCaret}
              onScroll={syncOverlayScroll}
              onPaste={event => {
                // The owner owns the policy: intercept-and-become-attachments
                // (preventDefault) or fall through to normal text insertion.
                onPaste?.(event);
              }}
              onKeyDown={event => {
                onKeyDown?.(event);
                if (event.defaultPrevented) return;
                // Read the caret live: `showSuggestion` can lag a click that has
                // not yet flushed through `onSelect`.
                if (event.key === "Tab" && suggestion && caretAtEnd() && onAcceptSuggestion) {
                  event.preventDefault();
                  onAcceptSuggestion();
                  return;
                }
                if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  if (canSend) onSubmit();
                }
              }}
              className={cn(
                "relative z-10 max-h-44 min-h-[28px] w-full resize-none bg-transparent text-[15px] leading-relaxed tracking-[-0.006em] text-foreground outline-none placeholder:text-muted-foreground",
                isHero ? "min-h-16 px-1 py-1" : "px-1 py-0.5",
              )}
            />
          </div>

          <div className="flex min-h-8 items-center justify-between gap-2">
            {/* Leading edge of the controls row: the model chip, then the access
                control behind a hairline divider. */}
            <div className="flex min-w-0 items-center gap-1.5">
              {modelControl}
              {modelControl && accessControl ? <span aria-hidden="true" className="h-4 w-px shrink-0 bg-border" /> : null}
              {accessControl}
            </div>

            <div className="flex items-center gap-1">
              {trailing}
              {onAttachFiles && <>
                <input ref={attachmentInput} type="file" accept="image/png,image/jpeg,image/webp,image/gif" multiple className="hidden" aria-label="Choose images" onChange={event => {
                  const files = Array.from(event.currentTarget.files ?? []);
                  event.currentTarget.value = "";
                  if (files.length) onAttachFiles(files);
                }} />
                <button type="button" disabled={locked} aria-label="Attach images" title="Attach images" onClick={() => attachmentInput.current?.click()} className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"><Paperclip className="h-4 w-4" aria-hidden="true" /></button>
              </>}
              <button
                type="button"
                className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors duration-150 hover:bg-accent hover:text-foreground active:scale-95 disabled:opacity-40"
                onClick={onPlusClick}
                disabled={disabled || !onPlusClick || !!plusUnavailableReason}
                aria-label={plusLabel}
                title={plusUnavailableReason ?? plusLabel}
              >
                {plusIcon === "paperclip"
                  ? <Paperclip className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />
                  : <Plus className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />}
              </button>
              {leading}
              {/* Stop and submit are separate actions, and while a turn is running
                  both are present: sending guidance must never read as cancelling
                  the work. */}
              {(working || stopping) && onStop && (
                <button
                  type="button"
                  onClick={onStop}
                  disabled={stopping}
                  className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border-card bg-card text-foreground transition-colors duration-150 active:scale-95 hover:bg-accent disabled:opacity-70"
                  aria-label={stopping ? "Stopping…" : "Stop"}
                >
                  <Square className="h-3.5 w-3.5 fill-current" aria-hidden="true" />
                </button>
              )}
              {(!working || steerable) && (
                <button
                  type="submit"
                  disabled={!canSend}
                  className={cn(
                    "inline-flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-full transition-opacity duration-150 active:scale-95",
                    steerable ? "px-3 text-[13px] font-medium" : "w-8",
                    canSend
                      ? "bg-primary text-primary-foreground hover:opacity-90"
                      : "bg-accent text-muted-foreground/70",
                  )}
                  aria-label={submitLabel}
                  title={activeAction === "queue" && working ? "Held until the current step finishes" : undefined}
                >
                  {steerable && <span>{submitLabel}</span>}
                  <ArrowUp className="h-4 w-4" strokeWidth={2.5} aria-hidden="true" />
                </button>
              )}
            </div>
          </div>
          </div>
        </form>
      </div>
      {footer && <div className="pt-1.5">{footer}</div>}
    </div>
  );
}
