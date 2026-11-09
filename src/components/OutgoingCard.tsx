/**
 * A message a chat drafted for someone else, as the transcript shows it.
 *
 * The agent's `outgoing_draft` call becomes this card. It shows where the
 * message is going, the text as the destination will show it, and what
 * Bridge's style rules found or already fixed. The person edits, fixes,
 * copies or discards it here. Nothing is sent from this card yet: Copy is
 * the way out, and a later slice adds Send behind an exact-text approval.
 *
 * Still being written: a quiet placeholder. Refused by the core: one muted
 * line that opens to the reason, which the agent also received.
 */

import { useCallback, useEffect, useState } from "react";
import { AlertCircle, AtSign, Check, ChevronRight, ClipboardCopy, Hash, ImageIcon, MessageSquareText, Pencil, Trash2, Wand2, X } from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { ConversationItem } from "../conversation";
import type { OutgoingAttachment, OutgoingDestination, OutgoingLintFinding, OutgoingMessage } from "../protocol/generated/protocol";
import { Markdown } from "./Markdown";
import { outgoingCallState, outgoingDraftId, outgoingRefusal } from "../transcript/outgoing";

function DestinationIcon({ destination }: { destination: OutgoingDestination }) {
  const Icon = destination.kind === "slackDm" ? AtSign
    : destination.kind === "slackChannel" || destination.kind === "slackThread" ? Hash
    : destination.kind === "clipboard" ? ClipboardCopy
    : MessageSquareText;
  return <Icon size={11} className="shrink-0" aria-hidden="true" />;
}

function Finding({ finding }: { finding: OutgoingLintFinding }) {
  const open = !finding.fixed;
  return (
    <li
      data-finding={finding.rule}
      data-fixed={finding.fixed || undefined}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px]",
        open ? "border-warning/40 text-foreground/85" : "border-border text-muted-foreground",
      )}
    >
      {open
        ? <AlertCircle size={11} className="shrink-0 text-warning" aria-hidden="true" />
        : <Check size={11} className="shrink-0" aria-hidden="true" />}
      {finding.message}{finding.line ? <span className="tabular-nums text-muted-foreground"> · line {finding.line}</span> : null}
    </li>
  );
}

function Placeholder() {
  return (
    <div data-outgoing-state="drafting" className="rounded-2xl border border-dashed border-border px-4 py-3 text-[12.5px] text-muted-foreground">
      <span className="animate-pulse">Drafting a message…</span>
    </div>
  );
}

function RefusedRow({ detail }: { detail?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div data-outgoing-state="refused" className="min-w-0 text-[12px] text-muted-foreground">
      <button
        type="button"
        className="flex min-h-7 items-center gap-2 rounded-md px-1.5 text-left transition-colors hover:bg-accent/50 disabled:cursor-default"
        aria-expanded={detail ? open : undefined}
        disabled={!detail}
        onClick={() => setOpen(!open)}
      >
        <MessageSquareText size={13} aria-hidden="true" />
        <span>Message not drafted: the reason went back to the agent</span>
        {detail && <ChevronRight size={12} className={cn("transition-transform", open && "rotate-90")} aria-hidden="true" />}
      </button>
      {open && detail && <pre className="ml-6 mt-1 max-h-60 overflow-auto whitespace-pre-wrap rounded-lg bg-code p-3 font-mono text-[11.5px] leading-relaxed">{detail}</pre>}
    </div>
  );
}

/** An image as a PNG blob, which is what the clipboard reliably takes. */
async function pngBlob(dataUrl: string, mime: string): Promise<Blob> {
  const source = await (await fetch(dataUrl)).blob();
  if (mime === "image/png") return source;
  const bitmap = await createImageBitmap(source);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
  return new Promise((resolve, reject) => canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error("could not copy the image")), "image/png"));
}

function Attachment({ draftId, index, attachment, readOnly, busy, onRemove, onError }: {
  draftId: string;
  index: number;
  attachment: OutgoingAttachment;
  readOnly: boolean;
  busy: boolean;
  onRemove: () => void;
  onError: (message: string) => void;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let live = true;
    bridgeApi.readOutgoingAttachment(draftId, index)
      .then(image => { if (live) setSrc(`data:${image.mime};base64,${image.dataBase64}`); })
      .catch(() => { if (live) setFailed(true); });
    return () => { live = false; };
  }, [draftId, index, attachment.path]);
  const copy = async () => {
    if (!src) return;
    try {
      const blob = await pngBlob(src, attachment.mime);
      await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : "could not copy the image");
    }
  };
  return (
    <li data-outgoing-attachment={attachment.name} className="flex w-36 flex-col overflow-hidden rounded-xl border border-border bg-background/60">
      <div className="relative grid h-24 shrink-0 place-items-center overflow-hidden bg-muted/40">
        {src
          ? <img src={src} alt={attachment.name} className="absolute inset-0 size-full object-cover" />
          : <ImageIcon size={18} className={cn("text-muted-foreground", !failed && "animate-pulse")} aria-hidden="true" />}
      </div>
      <div className="flex min-w-0 items-center gap-1 px-2 py-1.5">
        <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/85" title={failed ? `${attachment.name}: file missing or changed` : attachment.name}>
          {failed ? "Missing: " : ""}{attachment.name}
        </span>
        {!readOnly && <>
          <button type="button" aria-label={copied ? "Image copied" : `Copy image ${attachment.name}`} disabled={!src || busy} onClick={() => void copy()}
            className="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
            {copied ? <Check size={12} aria-hidden="true" /> : <ClipboardCopy size={12} aria-hidden="true" />}
          </button>
          <button type="button" aria-label={`Remove image ${attachment.name}`} disabled={busy} onClick={onRemove}
            className="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
            <X size={12} aria-hidden="true" />
          </button>
        </>}
      </div>
    </li>
  );
}

/** The card for one stored draft. Exported for tests and other surfaces. */
export function OutgoingDraft({ draft, readOnly = false, onChange }: {
  draft: OutgoingMessage;
  readOnly?: boolean;
  onChange: (next: OutgoingMessage) => void;
}) {
  const [editing, setEditing] = useState(false);
  // The revision the editor opened at. A refresh while editing must not
  // move it, or a save would overwrite the change that caused the refresh.
  const [editBase, setEditBase] = useState(draft.revision);
  const [text, setText] = useState(draft.body);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async (action: () => Promise<OutgoingMessage>) => {
    setBusy(true);
    setError(null);
    try {
      onChange(await action());
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return false;
    } finally {
      setBusy(false);
    }
  }, [onChange]);

  if (draft.state === "discarded") {
    return (
      <div data-outgoing-state="discarded" className="flex min-h-7 items-center gap-2 px-1.5 text-[12px] text-muted-foreground">
        <Trash2 size={13} aria-hidden="true" />
        <span>Draft discarded</span>
        <span aria-hidden="true">·</span>
        <span className="truncate">{draft.label}</span>
      </div>
    );
  }

  const attachments = draft.attachments ?? [];
  const canFix = draft.lint.some(finding => finding.fixable && !finding.fixed);
  const save = async () => {
    if (await run(async () => (await bridgeApi.updateOutgoing(draft.id, editBase, text)).item)) setEditing(false);
  };
  const copy = () => run(async () => {
    const result = await bridgeApi.copyOutgoing(draft.id, draft.revision);
    await navigator.clipboard?.writeText(result.text);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
    return result.item;
  });

  return (
    <section
      data-outgoing-state={draft.state}
      aria-label={`Drafted message for ${draft.label}`}
      className="relative overflow-hidden rounded-2xl border border-border-card bg-card shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-16px_rgb(0_0_0/0.18)]"
    >
      <div className="px-4 pb-3.5 pt-3.5 sm:px-5">
        <p className="flex flex-wrap items-center gap-1.5 text-[10.5px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
          <span>Draft message</span>
          {draft.state === "copied" && <>
            <span aria-hidden="true" className="text-muted-foreground/50">·</span>
            <span className="normal-case tracking-normal">copied</span>
          </>}
        </p>
        <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
          <span data-outgoing-destination className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-border bg-background/60 px-1.5 py-0.5 text-[11px] text-foreground/85">
            <DestinationIcon destination={draft.destination} />
            <span className="truncate">{draft.label}</span>
          </span>
          {draft.reason && <span className="min-w-0 truncate">{draft.reason}</span>}
        </div>

        {editing ? (
          <textarea
            aria-label="Message text"
            className="mt-3 block min-h-32 w-full resize-y rounded-xl border border-input bg-background px-3 py-2.5 text-[13.5px] leading-relaxed text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
            value={text}
            onChange={event => setText(event.target.value)}
            disabled={busy}
          />
        ) : (
          // Markdown destinations (Slack through its connector, GitHub) are
          // previewed formatted; pasted mrkdwn and plain text as typed.
          draft.format === "slackMarkdown" || draft.format === "github" ? (
            <div data-outgoing-preview className="mt-3 max-h-96 overflow-auto rounded-xl bg-background/60 px-3 py-2.5 text-[13.5px] leading-relaxed text-foreground">
              <Markdown text={draft.rendered} />
            </div>
          ) : (
            <div data-outgoing-preview className="mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-xl bg-background/60 px-3 py-2.5 text-[13.5px] leading-relaxed text-foreground">
              {draft.rendered}
            </div>
          )
        )}

        {attachments.length > 0 && (
          <ul className="mt-2.5 flex flex-wrap gap-2" aria-label="Images">
            {attachments.map((attachment, index) => (
              <Attachment
                key={`${attachment.path}-${index}`}
                draftId={draft.id}
                index={index}
                attachment={attachment}
                readOnly={readOnly}
                busy={busy}
                onError={setError}
                onRemove={() => void run(async () => (await bridgeApi.updateOutgoing(
                  draft.id, draft.revision, undefined, false,
                  attachments.filter((_, other) => other !== index).map(kept => kept.path),
                )).item)}
              />
            ))}
          </ul>
        )}

        {draft.lint.length > 0 && !editing && (
          <ul className="mt-2.5 flex flex-wrap gap-1.5" aria-label="Style checks">
            {draft.lint.map((finding, index) => <Finding key={`${finding.rule}-${finding.line ?? "x"}-${index}`} finding={finding} />)}
          </ul>
        )}

        {error && <p role="alert" className="mt-2.5 text-[12px] text-destructive">{error}</p>}

        {!readOnly && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {editing ? <>
              <Button size="sm" onClick={() => void save()} disabled={busy || !text.trim()}>Save</Button>
              <Button size="sm" variant="ghost" onClick={() => { setEditing(false); setText(draft.body); setError(null); }} disabled={busy}>Cancel</Button>
            </> : <>
              <Button size="sm" onClick={() => void copy()} disabled={busy}>
                {copied ? <Check aria-hidden="true" /> : <ClipboardCopy aria-hidden="true" />}
                {copied ? "Copied" : "Copy"}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => { setText(draft.body); setEditBase(draft.revision); setEditing(true); }} disabled={busy}>
                <Pencil aria-hidden="true" />Edit
              </Button>
              {canFix && (
                <Button size="sm" variant="outline" onClick={() => void run(async () => (await bridgeApi.updateOutgoing(draft.id, draft.revision, undefined, true)).item)} disabled={busy}>
                  <Wand2 aria-hidden="true" />Fix
                </Button>
              )}
              <Button size="sm" variant="ghost" className="ml-auto text-muted-foreground" onClick={() => void run(async () => (await bridgeApi.discardOutgoing(draft.id)).item)} disabled={busy}>
                <Trash2 aria-hidden="true" />Discard
              </Button>
            </>}
          </div>
        )}
      </div>
    </section>
  );
}

/**
 * A transcript row for an `outgoing_draft` call. Loads the stored draft and
 * follows it: the card shows the draft as it is now, not as it was drafted.
 */
export function OutgoingCard({ item, sessionId, readOnly = false }: { item: ConversationItem; sessionId?: string; readOnly?: boolean }) {
  const state = outgoingCallState(item);
  const draftId = outgoingDraftId(item);
  const [draft, setDraft] = useState<OutgoingMessage | null>(null);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    if (!draftId || !sessionId) return;
    let live = true;
    const load = () => bridgeApi.listOutgoing(sessionId).then(result => {
      if (!live) return;
      const found = result.items.find(candidate => candidate.id === draftId) ?? null;
      setDraft(found);
      setMissing(!found);
    }).catch(() => { if (live) setMissing(true); });
    void load();
    const unlisten = bridgeApi.onOutgoingChanged(payload => {
      if (payload.sessionId === sessionId && payload.id === draftId) void load();
    });
    return () => {
      live = false;
      void unlisten.then(stop => stop());
    };
  }, [draftId, sessionId]);

  if (state === "drafting") return <Placeholder />;
  if (state === "refused") return <RefusedRow detail={outgoingRefusal(item)} />;
  if (!draft) {
    return missing
      ? <div data-outgoing-state="missing" className="px-1.5 text-[12px] text-muted-foreground">This drafted message is no longer stored.</div>
      : <Placeholder />;
  }
  return <OutgoingDraft draft={draft} readOnly={readOnly} onChange={setDraft} />;
}
