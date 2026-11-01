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
      const text = markdown.replace(/\r\n?/g, "\n");
      let tokens: { type: string; raw: string }[];
      try { tokens = new manager.instance.Lexer(manager.instance.defaults).lex(text); } catch { return null; }
      const blocks: string[] = [];
      let lead = "";
      for (const token of tokens) {
        if (token.type === "space") {
          if (blocks.length) blocks[blocks.length - 1] += token.raw;
          else lead += token.raw;
          continue;
        }
        blocks.push(lead + token.raw);
        lead = "";
      }
      if (lead) blocks.length ? (blocks[blocks.length - 1] += lead) : blocks.push(lead);
      return blocks.join("") === text ? blocks : null;
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
