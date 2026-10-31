// The Notes editor's bookkeeping, kept pure so it can be tested without a DOM.
//
// A save sends the full text with the seq it came from and the core merges it
// with whatever landed since. The one case the core cannot see is typing that
// happens while a save is in flight: that typing is derived from the text that
// was sent, not from any version the core stored. `rebaseDraft` replays it onto
// the merged result so the next save carries a real base.

import type { Note } from "./types";

function commonPrefix(a: string, b: string): number {
  const limit = Math.min(a.length, b.length);
  let index = 0;
  while (index < limit && a.charCodeAt(index) === b.charCodeAt(index)) index += 1;
  return index;
}

/** Shared tail length, never reaching into the first `floor` characters. */
function commonSuffix(a: string, b: string, floor: number): number {
  const limit = Math.min(a.length, b.length) - floor;
  let length = 0;
  while (length < limit && a.charCodeAt(a.length - 1 - length) === b.charCodeAt(b.length - 1 - length)) length += 1;
  return length;
}

export interface Rebased {
  text: string;
  /** The typing and the concurrent change touched the same span; `text` is the
   *  person's draft, since their words win on screen. */
  overlap: boolean;
}

/** Replay the edit `sent -> draft` onto `merged`, which is `sent` plus a
 *  concurrent change. */
export function rebaseDraft(sent: string, draft: string, merged: string): Rebased {
  if (draft === sent) return { text: merged, overlap: false };
  if (merged === sent) return { text: draft, overlap: false };
  const userStart = commonPrefix(sent, draft);
  const userTail = commonSuffix(sent, draft, userStart);
  const userEnd = sent.length - userTail;
  const inserted = draft.slice(userStart, draft.length - userTail);
  const otherStart = commonPrefix(sent, merged);
  const otherEnd = sent.length - commonSuffix(sent, merged, otherStart);
  if (userEnd <= otherStart) {
    return { text: merged.slice(0, userStart) + inserted + merged.slice(userEnd), overlap: false };
  }
  if (userStart >= otherEnd) {
    const shift = merged.length - sent.length;
    return { text: merged.slice(0, userStart + shift) + inserted + merged.slice(userEnd + shift), overlap: false };
  }
  return { text: draft, overlap: true };
}

/** What a page is called in a list: its title, else its first line. */
export function noteDisplayTitle(note: Pick<Note, "title" | "preview">): string {
  const title = note.title?.trim();
  if (title) return title;
  const line = note.preview.split("\n").map(part => part.replace(/^#+\s*/, "").trim()).find(Boolean);
  if (!line) return "Untitled";
  return line.length > 80 ? `${line.slice(0, 79)}…` : line;
}

/** Markdown as one line of plain text, for a list preview. */
export function plainPreview(markdown: string): string {
  return markdown
    .replace(/```[\s\S]*?(```|$)/g, " ")
    .replace(/^\s{0,3}(#{1,6}\s+|>\s?|[-*+]\s+(\[[ xX]\]\s+)?|\d+[.)]\s+)/gm, "")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/(^|\s)[*_](\S[^*_]*\S|\S)[*_](?=\s|$|[.,;:!?])/g, "$1$2")
    .replace(/\s+/g, " ")
    .trim();
}

/** A quoted line without the list marker it was typed with. */
export function quoteText(excerpt: string): string {
  return excerpt.trim().replace(/^(?:[-*+]|\d+[.)])\s+/, "");
}
