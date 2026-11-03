// How plain text is written into the page's Markdown. Tiptap escapes every
// `* _ [ ] ~ \` and turns `& < >` into entities, so typing "Q&A" or
// "snake_case" would put "Q&amp;A" and "snake\_case" in the file. Here a
// character is escaped only where Markdown would otherwise read it as syntax,
// so the file reads the way the words were typed.

import type { Editor, JSONContent } from "@tiptap/core";
import { INLINE_MATH } from "./blocks";

const PUNCTUATION = /[!-/:-@[-`{-~]/;
const WORD = /[\p{L}\p{N}]/u;

/** Escape `text` so Markdown reads it back as the same characters.
 *  `lineStart` says the text begins a line of its block. */
export function escapeText(text: string, lineStart: boolean): string {
  let out = "";
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    const before = text[index - 1];
    const after = text[index + 1];
    const startsLine = index === 0 ? lineStart : before === "\n";
    if (startsLine) {
      const rest = text.slice(index);
      if (/^(#{1,6}(\s|$)|[-+*]\s|>|=+\s*$|-+\s*$|\d{1,9}[.)](\s|$))/.test(rest)) {
        const digits = /^\d+/.exec(rest)?.[0];
        if (digits) { out += `${digits}\\`; index += digits.length - 1; continue; }
        out += `\\${char}`;
        continue;
      }
    }
    switch (char) {
      case "\\":
        out += after === undefined || PUNCTUATION.test(after) ? "\\\\" : char;
        break;
      case "`":
        out += "\\`";
        break;
      case "*":
        out += (!before || /\s/.test(before)) && (!after || /\s/.test(after)) ? char : "\\*";
        break;
      case "_":
        out += before && after && WORD.test(before) && WORD.test(after) ? char : "\\_";
        break;
      case "~":
        out += text.indexOf("~", index + 1) >= 0 || text.lastIndexOf("~", index - 1) >= 0 ? "\\~" : char;
        break;
      case "[":
        out += /\]/.test(text.slice(index + 1)) ? "\\[" : char;
        break;
      case "<":
        out += after && /[A-Za-z/!?]/.test(after) ? "&lt;" : char;
        break;
      case "&":
        out += /^&(#\d+|#x[0-9a-f]+|[a-z][a-z0-9]*);/i.test(text.slice(index)) ? "&amp;" : char;
        break;
      case "$":
        out += INLINE_MATH.test(text.slice(index)) || after === "$" ? "\\$" : char;
        break;
      default:
        out += char;
    }
  }
  return out;
}

type Encode = (text: string, node: JSONContent, parent?: JSONContent) => string;

/** Swap Tiptap's text encoding for `escapeText`. Code keeps its characters. */
export function installMinimalEscaping(editor: Editor) {
  const manager = editor.markdown as unknown as { encodeTextForMarkdown?: Encode; codeTypes?: Set<string> } | undefined;
  if (!manager?.encodeTextForMarkdown) return;
  const codeTypes = manager.codeTypes ?? new Set(["code", "codeBlock"]);
  manager.encodeTextForMarkdown = (text, node, parent) => {
    const inCode = (parent?.type != null && codeTypes.has(parent.type)) || (node.marks ?? []).some(mark => codeTypes.has(typeof mark === "string" ? mark : mark.type));
    return inCode ? text : escapeText(text, parent?.content?.[0] === node);
  };
}
