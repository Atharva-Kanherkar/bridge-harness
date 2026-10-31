// The Notes editor's bookkeeping, kept pure so it can be tested without a DOM.
//
// A save sends the full text with the seq it came from and the core merges it
// with whatever landed since. The one case the core cannot see is typing that
// happens while a save is in flight: that typing is derived from the text that
// was sent, not from any version the core stored. `rebaseDraft` replays it onto
// the merged result so the next save carries a real base.

import type { Note } from "./types";

/** Base tokens `[start, end)` replaced by `tokens`. */
interface Hunk { start: number; end: number; tokens: string[] }

/** Past this many edits a diff stops looking for the smallest script and
 *  reports one hunk over the changed region, which only makes a merge more
 *  cautious (a wider span counts as touched). */
const MAX_EDITS = 2000;

/** Myers' diff of two token lists, as replacement hunks over `a`. */
function diff(a: string[], b: string[]): Hunk[] {
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head += 1;
  let endA = a.length;
  let endB = b.length;
  while (endA > head && endB > head && a[endA - 1] === b[endB - 1]) { endA -= 1; endB -= 1; }
  const A = a.slice(head, endA);
  const B = b.slice(head, endB);
  if (!A.length && !B.length) return [];
  if (!A.length || !B.length || A.length + B.length > MAX_EDITS * 2) return [{ start: head, end: endA, tokens: B }];
  const trace: Map<number, number>[] = [];
  let v = new Map<number, number>([[1, 0]]);
  let found = -1;
  for (let d = 0; d <= Math.min(A.length + B.length, MAX_EDITS) && found < 0; d += 1) {
    trace.push(v);
    v = new Map(v);
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && (v.get(k - 1) ?? -1) < (v.get(k + 1) ?? -1)) ? v.get(k + 1) ?? 0 : (v.get(k - 1) ?? 0) + 1;
      let y = x - k;
      while (x < A.length && y < B.length && A[x] === B[y]) { x += 1; y += 1; }
      v.set(k, x);
      if (x >= A.length && y >= B.length) { found = d; break; }
    }
  }
  if (found < 0) return [{ start: head, end: endA, tokens: B }];
  // Walk back from the end, collecting which tokens of A were kept.
  const kept: [number, number][] = [];
  let x = A.length;
  let y = B.length;
  for (let d = found; d > 0; d -= 1) {
    const before = trace[d];
    const k = x - y;
    const prevK = k === -d || (k !== d && (before.get(k - 1) ?? -1) < (before.get(k + 1) ?? -1)) ? k + 1 : k - 1;
    const prevX = before.get(prevK) ?? 0;
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) { x -= 1; y -= 1; kept.push([x, y]); }
    x = prevX;
    y = prevY;
  }
  while (x > 0 && y > 0) { x -= 1; y -= 1; kept.push([x, y]); }
  kept.reverse();
  const hunks: Hunk[] = [];
  let ai = 0;
  let bi = 0;
  for (const [ka, kb] of [...kept, [A.length, B.length] as [number, number]]) {
    if (ka > ai || kb > bi) hunks.push({ start: head + ai, end: head + ka, tokens: B.slice(bi, kb) });
    ai = ka + 1;
    bi = kb + 1;
  }
  return hunks;
}

function apply(base: string[], hunks: Hunk[], from: number, to: number): string {
  let out = "";
  let at = from;
  for (const hunk of hunks) {
    out += base.slice(at, hunk.start).join("") + hunk.tokens.join("");
    at = hunk.end;
  }
  return out + base.slice(at, to).join("");
}

const lines = (text: string) => text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
const words = (text: string) => text.match(/\s+|[^\s]+/g) ?? [];

/** Three-way merge of token lists: every change on either side that the other
 *  did not touch is kept. Where both changed the same span, `onConflict`
 *  decides; it returns null when it cannot merge, and the person's side wins. */
function merge3(base: string[], mine: string[], theirs: string[], onConflict: (base: string, mine: string, theirs: string) => string | null): Rebased {
  const tagged = [
    ...diff(base, theirs).map(hunk => ({ ...hunk, mine: false })),
    ...diff(base, mine).map(hunk => ({ ...hunk, mine: true })),
  ].sort((a, b) => a.start - b.start || a.end - b.end || Number(a.mine) - Number(b.mine));
  let out = "";
  let at = 0;
  let overlap = false;
  for (let index = 0; index < tagged.length;) {
    const group = [tagged[index]];
    let end = tagged[index].end;
    const start = tagged[index].start;
    index += 1;
    // Spans clash only by sharing a token (or an insertion landing inside the
    // other's span); meeting at a boundary is not a clash.
    while (index < tagged.length && tagged[index].start < end) {
      end = Math.max(end, tagged[index].end);
      group.push(tagged[index]);
      index += 1;
    }
    out += base.slice(at, start).join("");
    const ours = group.filter(hunk => hunk.mine);
    const others = group.filter(hunk => !hunk.mine);
    if (!ours.length || !others.length) {
      out += apply(base, group, start, end);
    } else {
      const mineText = apply(base, ours, start, end);
      const theirsText = apply(base, others, start, end);
      const merged = mineText === theirsText ? mineText : onConflict(base.slice(start, end).join(""), mineText, theirsText);
      if (merged === null) overlap = true;
      out += merged ?? mineText;
    }
    at = end;
  }
  return { text: out + base.slice(at).join(""), overlap };
}

export interface Rebased {
  text: string;
  /** The typing and the concurrent change touched the same words; that span
   *  keeps the person's version, every other change is kept from both. */
  overlap: boolean;
}

/** Replay the edit `sent -> draft` onto `merged`, which is `sent` plus a
 *  concurrent change: lines first, then words inside lines both sides touched. */
export function rebaseDraft(sent: string, draft: string, merged: string): Rebased {
  if (draft === sent) return { text: merged, overlap: false };
  if (merged === sent) return { text: draft, overlap: false };
  let overlap = false;
  const result = merge3(lines(sent), lines(draft), lines(merged), (base, mine, theirs) => {
    const inner = merge3(words(base), words(mine), words(theirs), () => null);
    overlap ||= inner.overlap;
    return inner.text;
  });
  return { text: result.text, overlap: overlap || result.overlap };
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
