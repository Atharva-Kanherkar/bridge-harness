// The `/` menu: every block the page can hold, filtered as you type, the way
// Notion's works. Also opened by a block handle's `+`.

import { forwardRef, useEffect, useImperativeHandle, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Extension, type Editor, type Range } from "@tiptap/core";
import Suggestion, { type SuggestionKeyDownProps, type SuggestionProps } from "@tiptap/suggestion";
import { PluginKey } from "@tiptap/pm/state";
import { ChevronRight, Code2, FileText, Heading1, Heading2, Heading3, Image as ImageIcon, List, ListOrdered, ListTodo, Minus, Quote, Sigma, SquareRadical, Table2, Text as TextIcon, MessageSquareText } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

/** What a block choice needs from the page around the editor. */
export interface BlockHost {
  pickImage: (at: number) => void;
  editMath: (kind: "inline" | "block", at: number) => void;
  createPage: (at: number) => void;
}

export interface BlockChoice {
  id: string;
  label: string;
  hint: string;
  keywords: string;
  icon: ReactNode;
  run: (editor: Editor, host: BlockHost) => void;
}

const icon = (Icon: typeof TextIcon) => <Icon size={16} strokeWidth={1.75} aria-hidden="true" />;

export const BLOCK_CHOICES: BlockChoice[] = [
  { id: "text", label: "Text", hint: "Plain writing", keywords: "paragraph plain", icon: icon(TextIcon), run: editor => editor.chain().focus().setParagraph().run() },
  { id: "h1", label: "Heading 1", hint: "Big section heading", keywords: "title h1 #", icon: icon(Heading1), run: editor => editor.chain().focus().setNode("heading", { level: 1 }).run() },
  { id: "h2", label: "Heading 2", hint: "Medium section heading", keywords: "subtitle h2 ##", icon: icon(Heading2), run: editor => editor.chain().focus().setNode("heading", { level: 2 }).run() },
  { id: "h3", label: "Heading 3", hint: "Small section heading", keywords: "subheading h3 ###", icon: icon(Heading3), run: editor => editor.chain().focus().setNode("heading", { level: 3 }).run() },
  { id: "bullet", label: "Bulleted list", hint: "A simple list", keywords: "ul unordered -", icon: icon(List), run: editor => editor.chain().focus().toggleBulletList().run() },
  { id: "number", label: "Numbered list", hint: "A list with numbers", keywords: "ol ordered 1.", icon: icon(ListOrdered), run: editor => editor.chain().focus().toggleOrderedList().run() },
  { id: "todo", label: "To-do list", hint: "Track things to do", keywords: "task checkbox check []", icon: icon(ListTodo), run: editor => editor.chain().focus().toggleTaskList().run() },
  { id: "toggle", label: "Toggle", hint: "Hide content inside", keywords: "details collapse fold", icon: icon(ChevronRight), run: editor => editor.chain().focus().setDetails().run() },
  { id: "quote", label: "Quote", hint: "Set a passage apart", keywords: "blockquote >", icon: icon(Quote), run: editor => editor.chain().focus().toggleBlockquote().run() },
  { id: "callout", label: "Callout", hint: "Make a note stand out", keywords: "note tip warning alert info", icon: icon(MessageSquareText), run: editor => editor.chain().focus().wrapIn("callout", { kind: "NOTE" }).run() },
  { id: "divider", label: "Divider", hint: "A line between sections", keywords: "hr rule separator ---", icon: icon(Minus), run: editor => editor.chain().focus().setHorizontalRule().run() },
  { id: "code", label: "Code", hint: "A block of code", keywords: "codeblock snippet ```", icon: icon(Code2), run: editor => editor.chain().focus().toggleCodeBlock().run() },
  { id: "table", label: "Table", hint: "Rows and columns", keywords: "grid", icon: icon(Table2), run: editor => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run() },
  { id: "image", label: "Image", hint: "Upload a picture", keywords: "photo picture screenshot png jpg", icon: icon(ImageIcon), run: (editor, host) => host.pickImage(editor.state.selection.from) },
  { id: "math", label: "Math block", hint: "An equation in TeX", keywords: "equation latex katex formula tex", icon: icon(SquareRadical), run: (editor, host) => host.editMath("block", editor.state.selection.from) },
  { id: "inline-math", label: "Inline math", hint: "TeX inside a line", keywords: "equation latex formula tex $", icon: icon(Sigma), run: (editor, host) => host.editMath("inline", editor.state.selection.from) },
  { id: "page", label: "Page", hint: "A page inside this one", keywords: "subpage child doc", icon: icon(FileText), run: (editor, host) => host.createPage(editor.state.selection.from) },
];

export function matchChoices(query: string): BlockChoice[] {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  if (!words.length) return BLOCK_CHOICES;
  return BLOCK_CHOICES.filter(choice => {
    const haystack = `${choice.label} ${choice.keywords}`.toLowerCase();
    return words.every(word => haystack.includes(word));
  });
}

export interface SlashState {
  query: string;
  range: Range;
  rect: DOMRect | null;
  choose: (choice: BlockChoice) => void;
}

export const slashKey = new PluginKey("noteSlash");

/** The `/` trigger. The menu itself is React, fed through `onState`. */
export function slashCommand(onState: (state: SlashState | null) => void, onKey: (event: KeyboardEvent) => boolean, host: () => BlockHost) {
  return Extension.create({
    name: "noteSlash",
    addProseMirrorPlugins() {
      return [Suggestion<BlockChoice, BlockChoice>({
        editor: this.editor,
        pluginKey: slashKey,
        char: "/",
        allowSpaces: false,
        startOfLine: false,
        allow: ({ state, range }) => {
          const $from = state.doc.resolve(range.from);
          return !$from.parent.type.spec.code && $from.parent.type.name !== "detailsSummary";
        },
        items: ({ query }) => matchChoices(query),
        command: ({ editor, range, props }) => {
          editor.chain().focus().deleteRange(range).run();
          props.run(editor, host());
        },
        render: () => {
          const show = (props: SuggestionProps<BlockChoice, BlockChoice>) => onState({ query: props.query, range: props.range, rect: props.clientRect?.() ?? null, choose: choice => props.command(choice) });
          return {
            onStart: show,
            onUpdate: show,
            onKeyDown: ({ event }: SuggestionKeyDownProps) => {
              if (event.key === "Escape") { onState(null); return true; }
              return onKey(event);
            },
            onExit: () => onState(null),
          };
        },
      })];
    },
  });
}

export interface SlashMenuHandle { onKeyDown: (event: KeyboardEvent) => boolean }

export const SlashMenu = forwardRef<SlashMenuHandle, { state: SlashState }>(function SlashMenu({ state }, ref) {
  const choices = matchChoices(state.query);
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setActive(0), [state.query]);
  useEffect(() => { list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" }); }, [active]);
  useImperativeHandle(ref, () => ({
    onKeyDown: event => {
      if (!choices.length) return false;
      if (event.key === "ArrowDown") { setActive(index => (index + 1) % choices.length); return true; }
      if (event.key === "ArrowUp") { setActive(index => (index - 1 + choices.length) % choices.length); return true; }
      if (event.key === "Enter" || event.key === "Tab") { state.choose(choices[Math.min(active, choices.length - 1)]); return true; }
      return false;
    },
  }), [choices, active, state]);
  const rect = state.rect;
  if (!rect) return null;
  const below = rect.bottom + 320 < window.innerHeight;
  // In the body: a transformed pane would make `fixed` relative to itself.
  return createPortal(<div
    ref={list}
    role="listbox"
    aria-label="Blocks"
    style={{ left: Math.min(rect.left, window.innerWidth - 300), top: below ? rect.bottom + 6 : undefined, bottom: below ? undefined : window.innerHeight - rect.top + 6 }}
    className="fixed z-50 max-h-80 w-72 overflow-y-auto rounded-xl bg-popover p-1 shadow-lg ring-1 ring-border"
    onMouseDown={event => event.preventDefault()}
  >
    {choices.length
      ? <>
        <p className="m-0 px-2 pb-1 pt-1.5 text-[11px] font-medium text-faint">{state.query ? "Results" : "Blocks"}</p>
        {choices.map((choice, index) => <button
          key={choice.id}
          type="button"
          role="option"
          data-index={index}
          aria-selected={index === active}
          onMouseEnter={() => setActive(index)}
          onClick={() => state.choose(choice)}
          className={clsx("flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left", index === active ? "bg-accent" : "")}
        >
          <span className="grid size-8 shrink-0 place-items-center rounded-md bg-card text-muted-foreground ring-1 ring-border">{choice.icon}</span>
          <span className="min-w-0">
            <span className="block truncate text-[13px] text-foreground">{choice.label}</span>
            <span className="block truncate text-[11.5px] text-muted-foreground">{choice.hint}</span>
          </span>
        </button>)}
      </>
      : <p className="m-0 px-2.5 py-2 text-[12.5px] text-muted-foreground">No blocks match “{state.query}”.</p>}
  </div>, document.body);
});
