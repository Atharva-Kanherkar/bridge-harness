// Every extension the page editor loads, in one place so the editor and the
// tests build the same schema and the same Markdown.

import type { AnyExtension } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "@tiptap/markdown";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { TableKit } from "@tiptap/extension-table";
import Image from "@tiptap/extension-image";
import { Placeholder } from "@tiptap/extensions";
import { Callout, NoteBlockMath, NoteInlineMath, PageLink, RawBlock, Toggle, ToggleContent, ToggleSummary } from "./blocks";

export interface NoteSchemaOptions {
  placeholder?: string;
  /** Click on a math formula: edit it. */
  onMathClick?: (kind: "inline" | "block", latex: string, pos: number) => void;
  /** Extra extensions (menus, node views) that change no Markdown. */
  extra?: AnyExtension[];
  image?: typeof Image;
  pageLink?: typeof PageLink;
  callout?: typeof Callout;
}

export function noteExtensions({ placeholder, onMathClick, extra = [], image = Image, pageLink = PageLink, callout = Callout }: NoteSchemaOptions = {}): AnyExtension[] {
  return [
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
      link: { openOnClick: false, autolink: true, linkOnPaste: true },
      dropcursor: { width: 2, class: "note-dropcursor" },
      trailingNode: {},
    }),
    Markdown.configure({ indentation: { style: "space", size: 2 }, markedOptions: { gfm: true } }),
    TaskList,
    TaskItem.configure({ nested: true }),
    TableKit.configure({ table: { resizable: false } }),
    image.configure({ inline: false, allowBase64: false }),
    NoteInlineMath.configure({ katexOptions: { throwOnError: false }, onClick: (node, pos) => onMathClick?.("inline", node.attrs.latex, pos) }),
    NoteBlockMath.configure({ katexOptions: { throwOnError: false, displayMode: true }, onClick: (node, pos) => onMathClick?.("block", node.attrs.latex, pos) }),
    callout,
    Toggle.configure({ persist: true, HTMLAttributes: { class: "note-toggle" } }),
    ToggleSummary,
    ToggleContent,
    pageLink,
    RawBlock,
    Placeholder.configure({
      placeholder: ({ node, editor }) => {
        if (node.type.name === "heading") return `Heading ${node.attrs.level}`;
        if (node.type.name === "detailsSummary") return "Toggle";
        return editor.isEmpty ? placeholder ?? "Write, or press / for blocks" : "Press / for blocks";
      },
      showOnlyCurrent: true,
      includeChildren: true,
    }),
    ...extra,
  ];
}
