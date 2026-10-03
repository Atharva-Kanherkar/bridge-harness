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
  return `${failed}${RESTORE_SEPARATOR}${current}`;
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
 * The composer once a retried send has taken its words back out of it.
 *
 * A retry (after signing in) resends a payload the failure had already put
 * back in the composer, alone or ahead of newer typing. That copy goes; the
 * newer typing, and anything else the composer holds, stays.
 */
export function withoutResentText(current: string, resent: string): string {
  if (!resent.trim()) return current;
  if (current.trim() === resent.trim()) return "";
  const prefix = `${resent}${RESTORE_SEPARATOR}`;
  return current.startsWith(prefix) ? current.slice(prefix.length) : current;
}

export function withoutResentAttachments(current: readonly ComposerAttachment[], resent: readonly ComposerAttachment[]): ComposerAttachment[] {
  const ids = new Set(resent.map(attachment => attachment.id));
  return current.filter(attachment => !ids.has(attachment.id));
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
