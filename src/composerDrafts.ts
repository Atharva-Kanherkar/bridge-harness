import type { ComposerAttachment } from "./pasteAttachments";

/** What the chat composer holds for one chat: words and pasted images. */
export interface ComposerDraft {
  text: string;
  attachments: ComposerAttachment[];
}

export const EMPTY_DRAFT: ComposerDraft = { text: "", attachments: [] };

export function isEmptyDraft(draft: ComposerDraft): boolean {
  return !draft.text.trim() && draft.attachments.length === 0;
}

/** Between restored words and the typing that came after them. */
const RESTORE_SEPARATOR = "\n\n";

/**
 * Put a failed send's words back without overwriting what the user typed since.
 *
 * Into an empty composer the failed words simply return. Otherwise they go
 * first, a blank line, then the newer typing: the order they were written in,
 * and nothing either side wrote is dropped.
 */
export function mergeFailedText(current: string, failed: string): string {
  if (!current.trim()) return failed;
  if (!failed.trim()) return current;
  return `${failed.trim()}${RESTORE_SEPARATOR}${current}`;
}

/** A failed send's images go back first; one already there is not added twice. */
export function mergeFailedAttachments(current: readonly ComposerAttachment[], failed: readonly ComposerAttachment[]): ComposerAttachment[] {
  const restored = new Set(failed.map(attachment => attachment.id));
  return [...failed, ...current.filter(attachment => !restored.has(attachment.id))];
}

export function mergeFailedSend(current: ComposerDraft, failed: ComposerDraft): ComposerDraft {
  return { text: mergeFailedText(current.text, failed.text), attachments: mergeFailedAttachments(current.attachments, failed.attachments) };
}

/**
 * The composer once a send has taken its words out of it.
 *
 * A send that finishes after an await (a shortcut opening its chat, a retry
 * after signing in) must not empty the composer wholesale: the user may have
 * typed on, or the failure may have put the words back ahead of newer typing.
 * Only the sent words go, and only as a whole leading run; anything after them,
 * or a draft that no longer starts with them, stays.
 */
export function withoutSentText(current: string, sent: string): string {
  const words = sent.trim();
  if (!words) return current;
  const typed = current.trimStart();
  if (!typed.startsWith(words)) return current;
  const rest = typed.slice(words.length);
  // "prev" never consumes the start of "previous".
  if (rest && !/^\s/.test(rest)) return current;
  return rest.replace(/^\s+/, "");
}

/** The composer's images once a send has taken its own; ones pasted since stay. */
export function withoutSentAttachments(current: readonly ComposerAttachment[], sent: readonly ComposerAttachment[]): ComposerAttachment[] {
  const ids = new Set(sent.map(attachment => attachment.id));
  return current.filter(attachment => !ids.has(attachment.id));
}

export function withoutSent(current: ComposerDraft, sent: ComposerDraft): ComposerDraft {
  return { text: withoutSentText(current.text, sent.text), attachments: withoutSentAttachments(current.attachments, sent.attachments) };
}

/**
 * Drafts of the chats not on screen. The one on screen lives in the composer's
 * own state; this holds the rest until their chat is opened again.
 *
 * Kept in memory only, like the composer itself: a restart starts clean.
 */
export class ComposerDrafts {
  private drafts = new Map<string, ComposerDraft>();

  /** Hold a chat's draft. An empty one is forgotten rather than stored. */
  put(sessionId: string, draft: ComposerDraft): void {
    if (isEmptyDraft(draft)) this.drafts.delete(sessionId);
    else this.drafts.set(sessionId, draft);
  }

  /** Hand a chat's draft back to the composer, which owns it from then on. */
  take(sessionId: string): ComposerDraft {
    const draft = this.drafts.get(sessionId) ?? EMPTY_DRAFT;
    this.drafts.delete(sessionId);
    return draft;
  }

  peek(sessionId: string): ComposerDraft {
    return this.drafts.get(sessionId) ?? EMPTY_DRAFT;
  }

  /** Drop drafts of chats that no longer exist, images included. */
  retain(sessionIds: ReadonlySet<string>): void {
    for (const id of this.drafts.keys()) if (!sessionIds.has(id)) this.drafts.delete(id);
  }

  get size(): number {
    return this.drafts.size;
  }
}
