// Keeping a person's Markdown as they wrote it while a block editor edits it.
//
// The editor writes Markdown in its own style (`-` bullets, `**bold**`, padded
// tables). Written as is, one typed letter would restyle the whole page and
// show as a change to every block in history and in the `.md` file. So the
// editor's Markdown is matched block by block against the text it was loaded
// from: a block whose content is the same is written back with its original
// bytes, and only the blocks the person changed take the editor's style.

import { diff } from "./notesDraft";

export interface BlockCodec {
  /** Split Markdown into top-level blocks, each with the blank lines after
   *  it, so joining them gives the text back; null if it cannot. */
  split: (markdown: string) => string[] | null;
  /** How the editor writes `block` after reading it. */
  canon: (block: string) => string;
}

const trimEnd = (text: string) => text.replace(/\s+$/, "");

/** The editor's `output`, with every block it left alone written as it was in
 *  `source`. */
export function keepUntouched(source: string, output: string, codec: BlockCodec): string {
  if (source === output) return output;
  const before = codec.split(source);
  const after = codec.split(output);
  if (!before || !after) return output;
  const original = before.map(block => codec.canon(block).trim());
  const written = after.map(block => block.trim());
  // Which source block each output block is, where it is one.
  const from = new Array<number | null>(written.length).fill(null);
  let a = 0;
  let b = 0;
  const pair = (until: number) => { for (; a < until; a += 1, b += 1) from[b] = a; };
  for (const hunk of diff(original, written)) {
    pair(hunk.start);
    a = hunk.end;
    b += hunk.tokens.length;
  }
  pair(original.length);
  // A block dragged elsewhere is a delete and an insert to the diff; it is
  // still the same block, so it keeps its bytes too.
  const unclaimed = new Map<string, number[]>();
  const claimed = new Set(from);
  original.forEach((block, index) => { if (!claimed.has(index)) unclaimed.set(block, [...(unclaimed.get(block) ?? []), index]); });
  written.forEach((block, index) => { if (from[index] === null) from[index] = unclaimed.get(block)?.shift() ?? null; });

  let text = "";
  written.forEach((block, index) => {
    const at = from[index];
    const piece = at === null ? block : before[at];
    if (index === written.length - 1) {
      // The page's last block keeps its own ending; a new last block ends the
      // way the page did.
      text += at === before.length - 1 ? piece : trimEnd(piece) + (source.endsWith("\n") ? "\n" : "");
      return;
    }
    // A kept block followed by the block that followed it before keeps its
    // own spacing; any other seam gets one blank line.
    text += at !== null && from[index + 1] === at + 1 ? piece : `${trimEnd(piece)}\n\n`;
  });
  return text;
}

/** A body that opens with its own title as a heading: the heading (with the
 *  blank lines after it) and the rest, so the title is not shown twice. */
export function splitTitleHeading(text: string, title: string | null | undefined): { head: string; body: string } {
  const heading = /^#[ \t]+(.+?)[ \t]*(?:\n[ \t]*)*(?:\n|$)/.exec(text);
  if (!heading || !title?.trim() || heading[1].trim() !== title.trim()) return { head: "", body: text };
  return { head: heading[0], body: text.slice(heading[0].length) };
}
