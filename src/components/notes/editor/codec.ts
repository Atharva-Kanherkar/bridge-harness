// The editor's side of `keepUntouched`: blocks split by the editor's own
// Markdown lexer (so a toggle or callout is one block), and each block's
// canonical form read through the editor's own parser and writer.

import type { Editor } from "@tiptap/core";
import type { BlockCodec } from "../../../notesBlocks";

interface Manager {
  instance: { Lexer: new (options: unknown) => { lex: (src: string) => { type: string; raw: string }[] }; defaults: unknown };
  parse: (markdown: string) => unknown;
  serialize: (doc: unknown) => string;
}

export function editorCodec(editor: Editor): BlockCodec {
  const manager = editor.markdown as unknown as Manager;
  const canon = new Map<string, string>();
  return {
    split: markdown => {
      const ends: number[] = [];
      let text = "";
      for (let index = 0; index < markdown.length; index += 1) {
        if (markdown[index] === "\r") {
          text += "\n";
          if (markdown[index + 1] === "\n") index += 1;
        } else {
          text += markdown[index];
        }
        ends.push(index + 1);
      }
      let tokens: { type: string; raw: string }[];
      try { tokens = new manager.instance.Lexer(manager.instance.defaults).lex(text); } catch { return null; }
      const lengths: number[] = [];
      let lead = 0;
      for (const token of tokens) {
        if (token.type === "space") {
          if (lengths.length) lengths[lengths.length - 1] += token.raw.length;
          else lead += token.raw.length;
          continue;
        }
        lengths.push(lead + token.raw.length);
        lead = 0;
      }
      if (lead) lengths.length ? (lengths[lengths.length - 1] += lead) : lengths.push(lead);
      if (lengths.reduce((sum, length) => sum + length, 0) !== text.length) return null;
      const blocks: string[] = [];
      let from = 0;
      let start = 0;
      for (const length of lengths) {
        from += length;
        const end = from ? ends[from - 1] : 0;
        blocks.push(markdown.slice(start, end));
        start = end;
      }
      return blocks.join("") === markdown ? blocks : null;
    },
    canon: block => {
      const key = block.trim();
      let written = canon.get(key);
      if (written === undefined) {
        written = manager.serialize(manager.parse(key));
        if (canon.size > 2000) canon.clear();
        canon.set(key, written);
      }
      return written;
    },
  };
}
