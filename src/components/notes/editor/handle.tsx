// The handle beside the block under the pointer: `+` adds a block below and
// opens the `/` menu; the grip drags the block, or opens its menu on click.

import { useEffect, useRef, useState, type RefObject } from "react";
import type { Editor } from "@tiptap/core";
import { NodeSelection } from "@tiptap/pm/state";
import { ArrowDown, ArrowUp, Copy, GripVertical, Plus, Trash2 } from "lucide-react";

interface Hover { index: number; pos: number; top: number; element: HTMLElement }

/** The top-level block at `y`, if any. */
function blockAt(editor: Editor, y: number): Omit<Hover, "top"> | null {
  const { doc } = editor.state;
  const children = Array.from(editor.view.dom.children) as HTMLElement[];
  let pos = 0;
  for (let index = 0; index < Math.min(children.length, doc.childCount); index += 1) {
    const rect = children[index].getBoundingClientRect();
    if (y >= rect.top - 6 && y <= rect.bottom + 6) return { index, pos, element: children[index] };
    pos += doc.child(index).nodeSize;
  }
  return null;
}

export function BlockHandle({ editor, area }: { editor: Editor; area: RefObject<HTMLElement | null> }) {
  const [hover, setHover] = useState<Hover | null>(null);
  const [menu, setMenu] = useState(false);
  const typing = useRef(false);

  useEffect(() => {
    const element = area.current;
    if (!element) return;
    const move = (event: MouseEvent) => {
      typing.current = false;
      if (menu || editor.view.dragging) return;
      const block = blockAt(editor, event.clientY);
      if (!block) return;
      const box = element.getBoundingClientRect();
      const rect = block.element.getBoundingClientRect();
      // Centered on the block's first line, so headings line up too.
      const line = Math.min(parseFloat(getComputedStyle(block.element).lineHeight) || 24, rect.height || 24);
      setHover({ ...block, top: rect.top - box.top + line / 2 - 12 });
    };
    const leave = () => { if (!menu) setHover(null); };
    const key = () => { typing.current = true; if (!menu) setHover(null); };
    element.addEventListener("mousemove", move);
    element.addEventListener("mouseleave", leave);
    editor.view.dom.addEventListener("keydown", key);
    return () => {
      element.removeEventListener("mousemove", move);
      element.removeEventListener("mouseleave", leave);
      editor.view.dom.removeEventListener("keydown", key);
    };
  }, [area, editor, menu]);

  if (!hover || !editor.isEditable) return null;
  const node = editor.state.doc.maybeChild(hover.index);
  if (!node) return null;
  const end = hover.pos + node.nodeSize;

  const add = () => {
    const empty = node.type.name === "paragraph" && node.content.size === 0;
    const chain = editor.chain().focus();
    if (empty) chain.setTextSelection(hover.pos + 1).insertContent("/").run();
    else chain.insertContentAt(end, { type: "paragraph" }).setTextSelection(end + 1).insertContent("/").run();
    setHover(null);
  };
  const select = () => editor.view.dispatch(editor.state.tr.setSelection(NodeSelection.create(editor.state.doc, hover.pos)));
  const dragStart = (event: React.DragEvent) => {
    const { view } = editor;
    select();
    const slice = view.state.selection.content();
    const { dom, text } = view.serializeForClipboard(slice);
    event.dataTransfer.clearData();
    event.dataTransfer.setData("text/html", dom.innerHTML);
    event.dataTransfer.setData("text/plain", text);
    event.dataTransfer.effectAllowed = "copyMove";
    event.dataTransfer.setDragImage(hover.element, 0, 0);
    view.dragging = { slice, move: true };
  };
  const moveBy = (step: -1 | 1) => {
    const { doc } = editor.state;
    const target = hover.index + step;
    if (target < 0 || target >= doc.childCount) return;
    const json = node.toJSON();
    const other = doc.child(target);
    const chain = editor.chain().focus().deleteRange({ from: hover.pos, to: end });
    const at = step < 0 ? hover.pos - other.nodeSize : hover.pos + other.nodeSize;
    chain.insertContentAt(at, json).run();
    setMenu(false);
    setHover(null);
  };
  const actions = [
    { label: "Move up", icon: ArrowUp, run: () => moveBy(-1), disabled: hover.index === 0 },
    { label: "Move down", icon: ArrowDown, run: () => moveBy(1), disabled: hover.index >= editor.state.doc.childCount - 1 },
    { label: "Duplicate", icon: Copy, run: () => { editor.chain().focus().insertContentAt(end, node.toJSON()).run(); setMenu(false); }, disabled: false },
    { label: "Delete", icon: Trash2, run: () => { editor.chain().focus().deleteRange({ from: hover.pos, to: end }).run(); setMenu(false); setHover(null); }, disabled: false },
  ];

  return <div data-block-handle="" style={{ top: hover.top }} className="absolute left-0 z-10 flex items-center" onMouseDown={event => { if ((event.target as HTMLElement).closest("button")) event.stopPropagation(); }}>
    <button type="button" aria-label="Add a block below" title="Add a block below" onClick={add} className="grid size-6 place-items-center rounded-md text-faint transition-colors hover:bg-accent hover:text-foreground">
      <Plus size={15} aria-hidden="true" />
    </button>
    <div className="relative">
      <button
        type="button"
        draggable
        aria-label="Drag to move, or click for options"
        title="Drag to move · click for options"
        aria-expanded={menu}
        onDragStart={dragStart}
        onDragEnd={() => setHover(null)}
        onClick={() => { select(); setMenu(open => !open); }}
        className="grid h-6 w-5 cursor-grab place-items-center rounded-md text-faint transition-colors hover:bg-accent hover:text-foreground active:cursor-grabbing"
      >
        <GripVertical size={15} aria-hidden="true" />
      </button>
      {menu && <>
        <div aria-hidden="true" className="fixed inset-0 z-40" onClick={() => { setMenu(false); setHover(null); }} />
        <div role="menu" aria-label="Block actions" className="absolute left-0 top-full z-50 mt-1 w-44 rounded-xl bg-popover p-1 shadow-lg ring-1 ring-border">
          {actions.map(action => <button key={action.label} type="button" role="menuitem" disabled={action.disabled} onClick={action.run} className="flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-[13px] text-foreground transition-colors hover:bg-accent disabled:opacity-40">
            <action.icon size={14} aria-hidden="true" className="text-muted-foreground" />{action.label}
          </button>)}
        </div>
      </>}
    </div>
  </div>;
}
