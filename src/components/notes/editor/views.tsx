// How images, child pages, embedded databases and wiki links draw inside the editor.

import { createContext, useContext, useEffect, useRef, useState, type FocusEvent, type ReactNode } from "react";
import Image from "@tiptap/extension-image";
import CodeBlock from "@tiptap/extension-code-block";
import { NodeViewContent, NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import { ArrowUpRight, FileText, Info, Lightbulb, MessageSquareWarning, OctagonAlert, Table2, TriangleAlert } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { assetUrl } from "./assets";
import { RichFence, richFenceKind } from "../../Markdown";
import { Callout, CALLOUT_KINDS, DatabaseEmbed, PageLink, WikiLink, type CalloutKind } from "./blocks";

/** What the views need from the page around the editor. */
export interface EditorPages {
  title: (id: string) => string | undefined;
  open: (id: string) => void;
  /** Open the page titled so, creating it first when none has that title. */
  openByTitle?: (title: string) => void;
  /** The pages a [[ menu can suggest, newest first. */
  pages?: () => { id: string; title: string }[];
  /** A database's live table, for a `<database>` block. */
  renderDatabase?: (id: string, view: string | null) => ReactNode;
}
export const EditorPagesContext = createContext<EditorPages>({ title: () => undefined, open: () => undefined });

const IMAGE_MIN_PX = 120;

/** The width the image's title holds ("420"), 0 or missing meaning natural. */
export function imageWidthOf(title: unknown): number | null {
  const parsed = title == null ? null : /^\d{1,4}$/.exec(String(title).trim());
  return parsed && Number(parsed[0]) > 0 ? Math.max(IMAGE_MIN_PX, Number(parsed[0])) : null;
}

/** The width a drag with a left or right handle lands on, clamped between
 *  120px and the column the editor sits in. */
export function draggedImageWidth(edge: "left" | "right", startX: number, width: number, x: number, available: number): number {
  const wanted = edge === "left" ? width + (startX - x) : width + (x - startX);
  return Math.round(Math.min(Math.max(wanted, IMAGE_MIN_PX), Math.max(IMAGE_MIN_PX, available)));
}

function ImageView({ node, updateAttributes, selected }: ReactNodeViewProps) {
  const src = String(node.attrs.src ?? "");
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [live, setLive] = useState<number | null>(null);
  const figure = useRef<HTMLElement>(null);
  const captionBox = useRef<HTMLElement>(null);
  // A drag in progress, dropped without saving if the view goes first.
  const dragging = useRef<(() => void) | null>(null);
  useEffect(() => () => dragging.current?.(), []);
  useEffect(() => {
    let current = true;
    setFailed(false);
    assetUrl(src).then(next => { if (current) setUrl(next); }, () => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [src]);
  // The title holds the width; a title that is words stays the image's own,
  // and that image keeps its natural size rather than lose it to a drag.
  const title = node.attrs.title == null ? "" : String(node.attrs.title);
  const resizable = !title.trim() || /^\d{1,4}$/.test(title.trim());
  const width = live ?? (resizable ? imageWidthOf(title) : null);
  const caption = String(node.attrs.alt ?? "");
  // The browser edits the caption's text, so React never renders into it:
  // the box takes the alt text only when it changes from outside.
  useEffect(() => {
    const box = captionBox.current;
    if (box && box.textContent !== caption) box.textContent = caption;
  }, [caption]);
  const commitCaption = (event: FocusEvent<HTMLElement>) => {
    // `![…]` holds the caption unescaped, so a bracket would end the image.
    const text = (event.currentTarget.textContent ?? "").replace(/\n/g, " ").replace(/[[\]]/g, "");
    if (event.currentTarget.textContent !== text) event.currentTarget.textContent = text;
    if (text !== caption) updateAttributes({ alt: text });
  };
  const beginDrag = (edge: "left" | "right") => (event: React.PointerEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const image = figure.current?.querySelector("img");
    if (!image) return;
    const imageRect = image.getBoundingClientRect();
    const start = {
      edge,
      x: event.clientX,
      width: imageRect.width,
      available: figure.current?.closest("article")?.clientWidth ?? imageRect.width,
    };
    let settled: number | null = null;
    const move = (dragEvent: PointerEvent) => {
      settled = draggedImageWidth(start.edge, start.x, start.width, dragEvent.clientX, start.available);
      setLive(settled);
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", cancel);
      dragging.current = null;
    };
    const finish = () => {
      stop();
      setLive(null);
      if (settled != null) updateAttributes({ title: String(settled) });
    };
    const cancel = () => { stop(); setLive(null); };
    dragging.current?.();
    dragging.current = stop;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", cancel);
  };
  const handle = clsx("absolute top-1/2 z-10 h-12 w-1.5 -translate-y-1/2 cursor-col-resize rounded-full bg-foreground/40 opacity-0 transition-opacity", selected ? "opacity-100" : "group-hover:opacity-100");
  return <NodeViewWrapper as="figure" ref={figure} data-drag-handle="" className="note-image group relative mx-auto my-3 w-fit max-w-full">
    <span className="relative block w-fit max-w-full">
      {failed
        ? <div className="rounded-lg bg-muted px-3 py-6 text-center text-[12.5px] text-muted-foreground">Couldn't load {src}</div>
        : url
          ? <img
            src={url}
            alt={caption}
            title={resizable ? undefined : title}
            draggable={false}
            // The width is the one computed thing a utility cannot say.
            style={width == null ? undefined : { width: `${width}px` }}
            className={clsx("mx-auto block max-h-[36rem] max-w-full rounded-lg", selected && "ring-2 ring-ring ring-offset-2 ring-offset-background")}
          />
          : <div aria-busy="true" className="h-40 animate-pulse rounded-lg bg-muted" />}
      {url && !failed && resizable && <>
        <button type="button" contentEditable={false} aria-label="Resize from the left edge" onPointerDown={beginDrag("left")} className={clsx(handle, "left-0 -translate-x-1/2")} />
        <button type="button" contentEditable={false} aria-label="Resize from the right edge" onPointerDown={beginDrag("right")} className={clsx(handle, "right-0 translate-x-1/2")} />
      </>}
    </span>
    <figcaption
      ref={captionBox}
      contentEditable
      suppressContentEditableWarning
      spellCheck="false"
      aria-label="Caption"
      onKeyDown={event => {
        if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); }
        // Select all means the caption, not the page around it.
        if (event.key === "a" && (event.metaKey || event.ctrlKey)) {
          event.preventDefault();
          getSelection()?.selectAllChildren(event.currentTarget);
        }
      }}
      onBlur={commitCaption}
      data-empty={caption ? undefined : ""}
      // An empty caption shows while the image is selected or in use, with a
      // prompt to type one.
      className={clsx("mt-1.5 text-center text-[12.5px] leading-relaxed text-muted-foreground outline-none empty:before:text-faint empty:before:content-['Add_a_caption'] focus-visible:bg-card focus-visible:ring-1 focus-visible:ring-ring", !selected && "data-[empty]:hidden group-focus-within:data-[empty]:block")}
    />
  </NodeViewWrapper>;
}

export const NoteImage = Image.extend({
  // The wrapper hugs the picture, so its handles and outline do too.
  addNodeView() { return ReactNodeViewRenderer(ImageView, { className: "mx-auto w-fit max-w-full" }); },
});

function PageLinkView({ node, selected }: ReactNodeViewProps) {
  const pages = useContext(EditorPagesContext);
  const id = String(node.attrs.id ?? "");
  const title = pages.title(id) ?? String(node.attrs.title ?? "") ?? "Untitled";
  return <NodeViewWrapper data-drag-handle="" className="my-0.5">
    <button
      type="button"
      contentEditable={false}
      onClick={() => pages.open(id)}
      className={clsx("-mx-1.5 flex w-[calc(100%+0.75rem)] items-center gap-2 rounded-md px-1.5 py-1 text-left text-[15px] text-foreground transition-colors hover:bg-accent", selected && "bg-selection")}
    >
      <FileText size={16} strokeWidth={1.75} aria-hidden="true" className="shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate underline decoration-border underline-offset-4">{title || "Untitled"}</span>
    </button>
  </NodeViewWrapper>;
}

export const NotePageLink = PageLink.extend({
  addNodeView() { return ReactNodeViewRenderer(PageLinkView); },
});

function DatabaseEmbedView({ node, selected }: ReactNodeViewProps) {
  const pages = useContext(EditorPagesContext);
  const id = String(node.attrs.id ?? "");
  const view = node.attrs.view == null ? null : String(node.attrs.view);
  const title = pages.title(id) ?? String(node.attrs.title ?? "");
  return <NodeViewWrapper data-drag-handle="" className={clsx("my-3 rounded-lg", selected && "ring-2 ring-ring ring-offset-2 ring-offset-background")}>
    <div contentEditable={false} className="mb-1.5 flex items-center gap-2">
      <Table2 size={15} strokeWidth={1.75} aria-hidden="true" className="shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate text-[15px] font-semibold text-foreground">{title || "Untitled database"}</span>
      <button type="button" onClick={() => pages.open(id)} className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-[12.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
        <ArrowUpRight size={13} aria-hidden="true" />Open as page
      </button>
    </div>
    <div contentEditable={false} data-database-frame="" className="max-h-[480px]">
      {pages.renderDatabase?.(id, view) ?? <p className="m-0 rounded-lg bg-muted px-3 py-4 text-[13px] text-muted-foreground">{title || "Untitled database"}</p>}
    </div>
  </NodeViewWrapper>;
}

export const NoteDatabaseEmbed = DatabaseEmbed.extend({
  // The table takes its own clicks and keys; only the title bar selects the block.
  addNodeView() {
    return ReactNodeViewRenderer(DatabaseEmbedView, {
      stopEvent: ({ event }) => !!(event.target as HTMLElement | null)?.closest?.("[data-database-frame]"),
    });
  },
});

function WikiLinkView({ node }: ReactNodeViewProps) {
  const pages = useContext(EditorPagesContext);
  // Spacing is kept in the Markdown, not in what the pill says or opens.
  const target = String(node.attrs.target ?? "").trim();
  const alias = node.attrs.alias == null ? null : String(node.attrs.alias).trim() || null;
  return <NodeViewWrapper as="span" className="inline">
    <button
      type="button"
      contentEditable={false}
      title={`Open the page ${target}`}
      aria-label={`Open the page ${target}`}
      onClick={() => { if (target) pages.openByTitle?.(target); }}
      className="inline-flex cursor-pointer items-baseline gap-1 rounded text-foreground align-baseline underline decoration-border underline-offset-4 transition-colors hover:decoration-foreground"
    >
      <FileText size={13} strokeWidth={1.75} aria-hidden="true" className="relative top-[1px] shrink-0 text-muted-foreground" />
      <span className="min-w-0 truncate">{alias ?? target}</span>
    </button>
  </NodeViewWrapper>;
}

export const NoteWikiLink = WikiLink.extend({
  addNodeView() { return ReactNodeViewRenderer(WikiLinkView); },
});

const CALLOUT_LOOK: Record<CalloutKind, { label: string; icon: typeof Info; tone: string }> = {
  NOTE: { label: "Note", icon: Info, tone: "bg-muted" },
  TIP: { label: "Tip", icon: Lightbulb, tone: "bg-muted" },
  IMPORTANT: { label: "Important", icon: MessageSquareWarning, tone: "bg-muted" },
  WARNING: { label: "Warning", icon: TriangleAlert, tone: "bg-warning/10" },
  CAUTION: { label: "Caution", icon: OctagonAlert, tone: "bg-destructive/10" },
};

function CalloutView({ node, updateAttributes }: ReactNodeViewProps) {
  const kind = (CALLOUT_KINDS.includes(node.attrs.kind) ? node.attrs.kind : "NOTE") as CalloutKind;
  const look = CALLOUT_LOOK[kind];
  const next = CALLOUT_KINDS[(CALLOUT_KINDS.indexOf(kind) + 1) % CALLOUT_KINDS.length];
  return <NodeViewWrapper as="aside" data-callout="" data-kind={kind} className={clsx("my-2 flex gap-2.5 rounded-lg px-3 py-2.5", look.tone)}>
    <button
      type="button"
      contentEditable={false}
      aria-label={`${look.label} callout. Change to ${CALLOUT_LOOK[next].label}`}
      title={`${look.label} · click to change`}
      onClick={() => updateAttributes({ kind: next })}
      className={clsx("mt-[0.3rem] grid size-5 shrink-0 place-items-center rounded transition-colors hover:bg-accent", kind === "WARNING" ? "text-warning" : kind === "CAUTION" ? "text-destructive" : "text-muted-foreground")}
    >
      <look.icon size={15} strokeWidth={1.9} aria-hidden="true" />
    </button>
    <NodeViewContent className="note-callout-body min-w-0 flex-1" />
  </NodeViewWrapper>;
}

export const NoteCallout = Callout.extend({
  addNodeView() { return ReactNodeViewRenderer(CalloutView); },
});

/** Whether the editor is focused with the cursor inside the node at `getPos`. */
function useCursorInside(editor: ReactNodeViewProps["editor"], getPos: ReactNodeViewProps["getPos"], size: number): boolean {
  const read = () => {
    const at = getPos();
    if (typeof at !== "number" || !editor.isFocused) return false;
    const { from, to } = editor.state.selection;
    return from > at && to < at + size;
  };
  const [inside, setInside] = useState(read);
  useEffect(() => {
    const update = () => setInside(read());
    update();
    editor.on("selectionUpdate", update);
    editor.on("focus", update);
    editor.on("blur", update);
    return () => { editor.off("selectionUpdate", update); editor.off("focus", update); editor.off("blur", update); };
  }, [editor, getPos, size]); // eslint-disable-line react-hooks/exhaustive-deps
  return inside;
}

// A fence chat draws as a figure (Mermaid, diagram, math, HTML) draws the same
// way here, through the chat's own renderer. The source stays in the document,
// hidden, and comes back while the cursor is in it.
function CodeBlockView({ node, editor, getPos }: ReactNodeViewProps) {
  const lang = String(node.attrs.language ?? "");
  const body = node.textContent;
  const inside = useCursorInside(editor, getPos, node.nodeSize);
  const figure = !!richFenceKind(lang) && body.trim() !== "" && !inside;
  const edit = (event: React.MouseEvent) => {
    // The figure's own buttons (copy, expand) keep their clicks.
    if ((event.target as Element).closest("button, a, iframe")) return;
    const at = getPos();
    if (typeof at !== "number") return;
    event.preventDefault();
    editor.chain().focus().setTextSelection(at + node.nodeSize - 1).run();
  };
  return <NodeViewWrapper data-language={lang || undefined}>
    {figure && <div contentEditable={false} data-rich-fence="" onMouseDown={edit} className="cursor-text">
      <RichFence lang={lang} body={body} />
    </div>}
    <pre hidden={figure}><NodeViewContent<"code"> as="code" className={lang ? `language-${lang}` : undefined} /></pre>
  </NodeViewWrapper>;
}

export const NoteCodeBlock = CodeBlock.extend({
  addNodeView() { return ReactNodeViewRenderer(CodeBlockView); },
});
