// The toolbar over selected text: marks, a link, math, and Turn into.

import { useState, type ReactNode } from "react";
import type { Editor } from "@tiptap/core";
import { useEditorState } from "@tiptap/react";
import { BubbleMenu } from "@tiptap/react/menus";
import { Bold, ChevronDown, Code, Italic, Link2, Sigma, Strikethrough, Underline } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { NodeSelection } from "@tiptap/pm/state";

const TURN_INTO: { id: string; label: string; active: (editor: Editor) => boolean; run: (editor: Editor) => void }[] = [
  { id: "text", label: "Text", active: editor => editor.isActive("paragraph") && !editor.isActive("bulletList") && !editor.isActive("orderedList") && !editor.isActive("taskList"), run: editor => editor.chain().focus().setParagraph().run() },
  { id: "h1", label: "Heading 1", active: editor => editor.isActive("heading", { level: 1 }), run: editor => editor.chain().focus().setNode("heading", { level: 1 }).run() },
  { id: "h2", label: "Heading 2", active: editor => editor.isActive("heading", { level: 2 }), run: editor => editor.chain().focus().setNode("heading", { level: 2 }).run() },
  { id: "h3", label: "Heading 3", active: editor => editor.isActive("heading", { level: 3 }), run: editor => editor.chain().focus().setNode("heading", { level: 3 }).run() },
  { id: "bullet", label: "Bulleted list", active: editor => editor.isActive("bulletList"), run: editor => editor.chain().focus().toggleBulletList().run() },
  { id: "number", label: "Numbered list", active: editor => editor.isActive("orderedList"), run: editor => editor.chain().focus().toggleOrderedList().run() },
  { id: "todo", label: "To-do list", active: editor => editor.isActive("taskList"), run: editor => editor.chain().focus().toggleTaskList().run() },
  { id: "quote", label: "Quote", active: editor => editor.isActive("blockquote"), run: editor => editor.chain().focus().toggleBlockquote().run() },
  { id: "code", label: "Code", active: editor => editor.isActive("codeBlock"), run: editor => editor.chain().focus().toggleCodeBlock().run() },
];

function Tool({ label, active = false, onClick, children }: { label: string; active?: boolean; onClick: () => void; children: ReactNode }) {
  return <button
    type="button"
    aria-label={label}
    aria-pressed={active}
    title={label}
    onMouseDown={event => event.preventDefault()}
    onClick={onClick}
    className={clsx("grid h-7 min-w-7 place-items-center rounded-md px-1 transition-colors", active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")}
  >{children}</button>;
}

export function SelectionToolbar({ editor, onMath }: { editor: Editor; onMath: () => void }) {
  const [linking, setLinking] = useState<string | null>(null);
  const [turning, setTurning] = useState(false);
  // Read on every transaction, so the buttons follow the selection.
  const active = useEditorState({
    editor,
    selector: ({ editor: current }) => ({
      block: TURN_INTO.find(choice => choice.active(current))?.label ?? "Text",
      bold: current.isActive("bold"),
      italic: current.isActive("italic"),
      underline: current.isActive("underline"),
      strike: current.isActive("strike"),
      code: current.isActive("code"),
      link: current.isActive("link"),
    }),
  });
  const current = active.block;
  const applyLink = (href: string) => {
    const chain = editor.chain().focus().extendMarkRange("link");
    if (href.trim()) chain.setLink({ href: /^[a-z][a-z0-9+.-]*:|^\/|^#/i.test(href.trim()) ? href.trim() : `https://${href.trim()}` }).run();
    else chain.unsetLink().run();
    setLinking(null);
  };
  return <BubbleMenu
    editor={editor}
    options={{ placement: "top", offset: 8 }}
    shouldShow={({ editor: current, state }) => {
      const { selection } = state;
      if (selection.empty || selection instanceof NodeSelection || !current.isEditable) return false;
      return !current.isActive("codeBlock") && state.doc.textBetween(selection.from, selection.to).trim().length > 0;
    }}
    className="z-40 flex items-center gap-0.5 rounded-xl bg-popover p-1 shadow-lg ring-1 ring-border"
  >
    {linking !== null
      ? <form onSubmit={event => { event.preventDefault(); applyLink(linking); }} className="flex items-center gap-1">
        <input
          autoFocus
          aria-label="Link address"
          value={linking}
          placeholder="Paste a link"
          onChange={event => setLinking(event.target.value)}
          onKeyDown={event => { if (event.key === "Escape") { event.preventDefault(); setLinking(null); editor.commands.focus(); } }}
          className="h-7 w-56 rounded-md bg-transparent px-2 text-[13px] text-foreground outline-none placeholder:text-faint"
        />
        {active.link && <button type="button" onClick={() => applyLink("")} className="h-7 rounded-md px-2 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground">Remove</button>}
      </form>
      : <>
        <div className="relative">
          <button type="button" aria-label="Turn into" aria-expanded={turning} onMouseDown={event => event.preventDefault()} onClick={() => setTurning(open => !open)} className="flex h-7 items-center gap-1 rounded-md px-2 text-[12.5px] text-foreground hover:bg-accent">
            {current}<ChevronDown size={12} aria-hidden="true" className="text-faint" />
          </button>
          {turning && <div role="menu" aria-label="Turn into" className="absolute left-0 top-full z-50 mt-1 w-44 rounded-xl bg-popover p-1 shadow-lg ring-1 ring-border">
            {TURN_INTO.map(choice => <button key={choice.id} type="button" role="menuitem" onMouseDown={event => event.preventDefault()} onClick={() => { choice.run(editor); setTurning(false); }} className={clsx("flex h-7 w-full items-center rounded-md px-2 text-left text-[12.5px] hover:bg-accent", choice.label === current ? "text-foreground" : "text-muted-foreground")}>{choice.label}</button>)}
          </div>}
        </div>
        <span aria-hidden="true" className="mx-0.5 h-4 w-px bg-border" />
        <Tool label="Bold" active={active.bold} onClick={() => editor.chain().focus().toggleBold().run()}><Bold size={14} /></Tool>
        <Tool label="Italic" active={active.italic} onClick={() => editor.chain().focus().toggleItalic().run()}><Italic size={14} /></Tool>
        <Tool label="Underline" active={active.underline} onClick={() => editor.chain().focus().toggleUnderline().run()}><Underline size={14} /></Tool>
        <Tool label="Strikethrough" active={active.strike} onClick={() => editor.chain().focus().toggleStrike().run()}><Strikethrough size={14} /></Tool>
        <Tool label="Code" active={active.code} onClick={() => editor.chain().focus().toggleCode().run()}><Code size={14} /></Tool>
        <Tool label="Link" active={active.link} onClick={() => setLinking(editor.getAttributes("link").href ?? "")}><Link2 size={14} /></Tool>
        <Tool label="Math" onClick={onMath}><Sigma size={14} /></Tool>
      </>}
  </BubbleMenu>;
}
