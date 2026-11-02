// How images, child pages and wiki links draw inside the editor.

import { createContext, useContext, useEffect, useState } from "react";
import Image from "@tiptap/extension-image";
import { NodeViewContent, NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import { FileText, Info, Lightbulb, MessageSquareWarning, OctagonAlert, TriangleAlert } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";
import { assetUrl } from "./assets";
import { Callout, CALLOUT_KINDS, PageLink, WikiLink, type CalloutKind } from "./blocks";

/** What the views need from the page around the editor. */
export interface EditorPages {
  title: (id: string) => string | undefined;
  open: (id: string) => void;
  /** Open the page titled so, creating it first when none has that title. */
  openByTitle?: (title: string) => void;
  /** The pages a [[ menu can suggest, newest first. */
  pages?: () => { id: string; title: string }[];
  /** Make a page titled so; null when it failed. */
  createByTitle?: (title: string) => Promise<{ id: string; title: string } | null>;
}
export const EditorPagesContext = createContext<EditorPages>({ title: () => undefined, open: () => undefined });

function ImageView({ node, selected }: ReactNodeViewProps) {
  const src = String(node.attrs.src ?? "");
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true;
    setFailed(false);
    assetUrl(src).then(next => { if (current) setUrl(next); }, () => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [src]);
  return <NodeViewWrapper as="figure" data-drag-handle="" className="note-image my-3">
    {failed
      ? <div className="rounded-lg bg-muted px-3 py-6 text-center text-[12.5px] text-muted-foreground">Couldn't load {src}</div>
      : url
        ? <img src={url} alt={String(node.attrs.alt ?? "")} title={node.attrs.title ?? undefined} draggable={false} className={clsx("mx-auto block max-h-[36rem] max-w-full rounded-lg", selected && "ring-2 ring-ring ring-offset-2 ring-offset-background")} />
        : <div aria-busy="true" className="h-40 animate-pulse rounded-lg bg-muted" />}
  </NodeViewWrapper>;
}

export const NoteImage = Image.extend({
  addNodeView() { return ReactNodeViewRenderer(ImageView); },
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

function WikiLinkView({ node }: ReactNodeViewProps) {
  const pages = useContext(EditorPagesContext);
  const target = String(node.attrs.target ?? "");
  const alias = node.attrs.alias == null ? null : String(node.attrs.alias);
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
