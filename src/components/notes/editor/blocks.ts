// The blocks plain Markdown has no form for, each written in a form the `.md`
// file still reads well in on GitHub, in Obsidian and in VS Code:
//   callout  > [!NOTE] then > text (a GitHub alert)
//   toggle   <details><summary>Title</summary> … </details>
//   page     <page url="bridge://p/<id>">Title</page> (a child page)
// Math, tables, to-dos and images come from Tiptap's own extensions; math is
// read more strictly than Tiptap does, so "$5 and $10" stays prose.

import { Node, mergeAttributes, type JSONContent, type MarkdownToken } from "@tiptap/core";
import { Details, DetailsContent, DetailsSummary } from "@tiptap/extension-details";
import { BlockMath, InlineMath } from "@tiptap/extension-mathematics";

/** `$x$`: no space inside either `$`, and no digit right after the closing
 *  one, the rule the chat renderer uses too. */
export const INLINE_MATH = /^\$(?![\s$])((?:\\.|[^\\$\n])+?)\$(?!\d)/;
const inlineMath = (src: string) => {
  const match = INLINE_MATH.exec(src);
  return match && !/\s$/.test(match[1]) ? match : null;
};

export const NoteInlineMath = InlineMath.extend({
  markdownTokenizer: {
    name: "inlineMath",
    level: "inline",
    start: (src: string) => src.indexOf("$"),
    tokenize: (src: string) => {
      const match = inlineMath(src);
      return match ? { type: "inlineMath", raw: match[0], latex: match[1] } : undefined;
    },
  },
});

/** `$$`, the TeX, `$$`, each on its own line; or `$$x$$` alone on a line. */
const BLOCK_MATH = /^ {0,3}\$\$[ \t]*\n([\s\S]+?)\n[ \t]*\$\$[ \t]*(?:\n|$)|^ {0,3}\$\$([^\n$]+)\$\$[ \t]*(?:\n|$)/;

export const NoteBlockMath = BlockMath.extend({
  markdownTokenizer: {
    name: "blockMath",
    level: "block",
    start: (src: string) => src.search(/^ {0,3}\$\$/m),
    tokenize: (src: string) => {
      const match = BLOCK_MATH.exec(src);
      return match ? { type: "blockMath", raw: match[0], latex: (match[1] ?? match[2]).trim() } : undefined;
    },
  },
});

export const CALLOUT_KINDS = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] as const;
export type CalloutKind = typeof CALLOUT_KINDS[number];

const CALLOUT_START = /^ {0,3}> ?\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*(?:\n|$)/i;
const QUOTED_LINE = /^ {0,3}>[^\n]*(?:\n|$)/;

/** Prefix every line with `> `, the way a quote holds blocks. */
const quoteLines = (text: string) => text.split("\n").map(line => line ? `> ${line}` : ">").join("\n");

export const Callout = Node.create({
  name: "callout",
  group: "block",
  content: "block+",
  defining: true,
  addAttributes() {
    return { kind: { default: "NOTE", parseHTML: element => element.getAttribute("data-kind") ?? "NOTE" } };
  },
  parseHTML() { return [{ tag: "aside[data-callout]" }]; },
  renderHTML({ node, HTMLAttributes }) {
    return ["aside", mergeAttributes(HTMLAttributes, { "data-callout": "", "data-kind": node.attrs.kind }), 0];
  },
  markdownTokenizer: {
    name: "callout",
    level: "block",
    start: (src: string) => src.search(/^ {0,3}> ?\[!/m),
    tokenize: (src: string, _tokens: MarkdownToken[], lexer) => {
      const head = CALLOUT_START.exec(src);
      if (!head) return undefined;
      let raw = head[0];
      const lines: string[] = [];
      for (let rest = src.slice(raw.length), line = QUOTED_LINE.exec(rest); line; rest = rest.slice(line[0].length), line = QUOTED_LINE.exec(rest)) {
        raw += line[0];
        lines.push(line[0].replace(/^ {0,3}> ?/, "").replace(/\n$/, ""));
      }
      return { type: "callout", raw, kind: head[1].toUpperCase(), tokens: lexer.blockTokens(lines.join("\n")) };
    },
  },
  parseMarkdown: (token, helpers) => {
    const children = helpers.parseChildren(token.tokens ?? []);
    return helpers.createNode("callout", { kind: token.kind }, children.length ? children : [helpers.createNode("paragraph")]);
  },
  renderMarkdown: (node, helpers) => {
    const written = String(node.attrs?.kind ?? "").toUpperCase();
    const kind = CALLOUT_KINDS.includes(written as CalloutKind) ? written : "NOTE";
    const body = helpers.renderChildren(node.content ?? [], "\n\n").replace(/\n+$/, "");
    return `> [!${kind}]\n${quoteLines(body)}`;
  },
});

/** Where a `<details>` block that starts `src` ends, nested ones included. */
function detailsEnd(src: string, from: number): { at: number; length: number } | null {
  const tags = /<\/?details\b[^>]*>/gi;
  tags.lastIndex = from;
  let depth = 1;
  for (let match = tags.exec(src); match; match = tags.exec(src)) {
    depth += match[0][1] === "/" ? -1 : 1;
    if (depth === 0) return { at: match.index, length: match[0].length };
  }
  return null;
}

export const Toggle = Details.extend({
  markdownTokenizer: {
    name: "details",
    level: "block",
    start: (src: string) => src.search(/^ {0,3}<details\b/im),
    tokenize: (src: string, _tokens: MarkdownToken[], lexer) => {
      const open = /^ {0,3}<details( open)?>[ \t]*\n?/i.exec(src);
      if (!open) return undefined;
      const close = detailsEnd(src, open[0].length);
      if (!close) return undefined;
      let end = close.at + close.length;
      while (src[end] === " " || src[end] === "\t") end += 1;
      if (src[end] === "\n") end += 1;
      const body = src.slice(open[0].length, close.at);
      const summary = /^\s*<summary>([\s\S]*?)<\/summary>[ \t]*\n?/i.exec(body);
      const rest = (summary ? body.slice(summary[0].length) : body).replace(/^\n+|\s+$/g, "");
      return {
        type: "details",
        raw: src.slice(0, end),
        open: !!open[1],
        summary: lexer.inlineTokens(summary ? summary[1].trim() : ""),
        tokens: rest ? lexer.blockTokens(rest) : [],
      };
    },
  },
  parseMarkdown: (token, helpers) => {
    const content = helpers.parseChildren(token.tokens ?? []);
    return helpers.createNode("details", {}, [
      helpers.createNode("detailsSummary", {}, helpers.parseInline(token.summary ?? [])),
      helpers.createNode("detailsContent", {}, content.length ? content : [helpers.createNode("paragraph")]),
    ]);
  },
  renderMarkdown: (node, helpers) => {
    const [summary, content] = (node.content ?? []) as JSONContent[];
    const title = summary ? helpers.renderChildren(summary.content ?? []) : "";
    const body = content ? helpers.renderChildren(content.content ?? [], "\n\n").replace(/\n+$/, "") : "";
    return `<details>\n<summary>${title}</summary>\n\n${body}\n\n</details>`;
  },
});
export { DetailsContent as ToggleContent, DetailsSummary as ToggleSummary };

const PAGE_TAG = /^ {0,3}<page url="bridge:\/\/p\/([^"\s]+)">([^<\n]*)<\/page>[ \t]*(?:\n|$)/;
const unescapeHtml = (text: string) => text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, "\"").replace(/&amp;/g, "&");
const escapeHtml = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** A child page, drawn as a link row. The node view is set by the editor. */
export const PageLink = Node.create({
  name: "pageLink",
  group: "block",
  atom: true,
  selectable: true,
  draggable: true,
  addAttributes() {
    return { id: { default: "" }, title: { default: "" } };
  },
  parseHTML() { return [{ tag: "div[data-page-link]", getAttrs: element => ({ id: element.getAttribute("data-page-link"), title: element.textContent ?? "" }) }]; },
  renderHTML({ node, HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { "data-page-link": node.attrs.id }), node.attrs.title || "Untitled"];
  },
  markdownTokenizer: {
    name: "pageLink",
    level: "block",
    start: (src: string) => src.search(/^ {0,3}<page url="bridge:/m),
    tokenize: (src: string) => {
      const match = PAGE_TAG.exec(src);
      return match ? { type: "pageLink", raw: match[0], id: match[1], title: unescapeHtml(match[2]) } : undefined;
    },
  },
  parseMarkdown: (token, helpers) => helpers.createNode("pageLink", { id: token.id, title: token.title }),
  renderMarkdown: node => `<page url="bridge://p/${node.attrs?.id ?? ""}">${escapeHtml(String(node.attrs?.title ?? ""))}</page>`,
});

// Markdown with no block here (a `<database>`, `<visual>` or `<chat>` tag, an
// HTML block, a comment) is kept as it was written and shown as source, so
// editing the page around it never drops or escapes it.
const RAW_START = /^ {0,3}<(?:!--|\/?(?:database|visual|chat|columns|column|div|table|figure|video|audio|iframe|section|picture|center|svg|img|p|pre|script|style|dl|form|h[1-6])\b)/i;

export const RawBlock = Node.create({
  name: "rawBlock",
  group: "block",
  atom: true,
  selectable: true,
  draggable: true,
  addAttributes() {
    return { source: { default: "" } };
  },
  parseHTML() { return [{ tag: "pre[data-raw-block]", getAttrs: element => ({ source: element.textContent ?? "" }) }]; },
  renderHTML({ node, HTMLAttributes }) {
    return ["pre", mergeAttributes(HTMLAttributes, { "data-raw-block": "", title: "Kept as written" }), node.attrs.source];
  },
  markdownTokenizer: {
    name: "rawBlock",
    level: "block",
    start: (src: string) => src.search(/^ {0,3}<[!a-z/]/im),
    tokenize: (src: string) => {
      if (!RAW_START.test(src)) return undefined;
      const comment = /^ {0,3}<!--[\s\S]*?-->[^\n]*(?:\n|$)/.exec(src);
      const block = comment ?? /^[\s\S]*?(?:\n[ \t]*\n|$)/.exec(src);
      if (!block) return undefined;
      const raw = block[0].replace(/\n[ \t]*\n$/, "\n");
      return { type: "rawBlock", raw, source: raw.replace(/\s+$/, "") };
    },
  },
  parseMarkdown: (token, helpers) => helpers.createNode("rawBlock", { source: token.source }),
  renderMarkdown: node => String(node.attrs?.source ?? ""),
});
