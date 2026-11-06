// Your words as a Notion-style page: blocks, a `/` menu, a selection
// toolbar, block handles, images and math, with no Markdown on screen.
//
// The page stays Markdown underneath. The editor hands its Markdown to the
// draft (at most every 250 ms while typing, and at once on `commit`), with
// every block the person did not touch kept byte for byte (`keepUntouched`).
// A change from outside (an agent, the `.md` file, a restore) replaces the
// content, or merges three ways with typing not yet handed over.

import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import type { Editor } from "@tiptap/core";
import { keepUntouched, splitTitleHeading, type BlockCodec } from "../../../notesBlocks";
import { rebaseDraft } from "../../../notesDraft";
import { assetRefusal, uploadAsset, ASSET_TYPES } from "./assets";
import { editorCodec } from "./codec";
import { noteExtensions } from "./extensions";
import { MathPopover, type MathEdit } from "./math";
import { installMinimalEscaping } from "./markdownText";
import { SlashMenu, slashCommand, type BlockHost, type SlashMenuHandle, type SlashState } from "./slash";
import { SelectionToolbar } from "./toolbar";
import { WikiMenu, wikiCommand, type WikiMenuHandle, type WikiState } from "./wiki";
import { BlockHandle } from "./handle";
import { EditorPagesContext, NoteCallout, NoteDatabaseEmbed, NoteImage, NotePageLink, NoteWikiLink, type EditorPages } from "./views";

export const HAND_OVER_MS = 250;

export interface NoteEditorHandle {
  /** Hand everything typed so far to `onChange` now. */
  commit: () => void;
  focus: (at?: "start" | "end") => void;
}

export interface NoteEditorProps {
  /** The page body as the draft holds it. */
  text: string;
  title: string | null;
  onChange: (text: string) => void;
  /** The editor lost focus, after handing over what was typed. */
  onLeave?: () => void;
  pages: EditorPages;
  /** Make a page titled so (null: untitled, the `/` menu's Page); resolves to
   *  it, or null if it failed. */
  onCreatePage: (title: string | null) => Promise<{ id: string; title: string } | null>;
  /** Make a database under this page (the `/` menu's Database); resolves to it, or null. */
  onCreateDatabase?: () => Promise<{ id: string; title: string } | null>;
  onError: (message: string) => void;
}

const imageFiles = (data: DataTransfer | null) => Array.from(data?.files ?? []).filter(file => file.type.startsWith("image/"));
const altText = (name: string) => name.replace(/\.[a-z0-9]+$/i, "").replace(/[[\]]/g, "");

export const NoteEditor = forwardRef<NoteEditorHandle, NoteEditorProps>(function NoteEditor(props, ref) {
  const latest = useRef(props);
  latest.current = props;
  // The title heading (if the body opens with one) stays out of the editor.
  const split = useRef(splitTitleHeading(props.text, props.title));
  /** The body the document was last in step with. */
  const source = useRef(split.current.body);
  /** The full text last handed out or taken in. */
  const emitted = useRef(props.text);
  const dirty = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const codec = useRef<BlockCodec | null>(null);
  const area = useRef<HTMLDivElement>(null);
  const files = useRef<HTMLInputElement>(null);
  const imageAt = useRef<number | null>(null);
  const slashMenu = useRef<SlashMenuHandle>(null);
  const [slash, setSlash] = useState<SlashState | null>(null);
  const wikiMenu = useRef<WikiMenuHandle>(null);
  const [wiki, setWiki] = useState<WikiState | null>(null);
  const [math, setMath] = useState<MathEdit | null>(null);
  const [uploading, setUploading] = useState<string | null>(null);
  const editorRef = useRef<Editor | null>(null);

  // Tiptap writes each empty paragraph after the first as a `&nbsp;` line;
  // the file keeps blank lines plain.
  const markdown = useCallback((editor: Editor) => keepUntouched(source.current, editor.getMarkdown().replace(/(^|\n)&nbsp;(?=\n|$)/g, "$1"), codec.current!), []);

  const commit = useCallback(() => {
    clearTimeout(timer.current);
    const editor = editorRef.current;
    if (!dirty.current || !editor || !codec.current) return;
    dirty.current = false;
    source.current = markdown(editor);
    const full = split.current.head + source.current;
    if (full === emitted.current) return;
    emitted.current = full;
    latest.current.onChange(full);
  }, [markdown]);

  const openMath = useCallback((kind: "inline" | "block", pos: number, latex: string, existing: boolean, replace?: { from: number; to: number }) => {
    const editor = editorRef.current;
    if (!editor) return;
    const dom = existing ? editor.view.nodeDOM(pos) : null;
    let rect: DOMRect | null = dom instanceof HTMLElement ? dom.getBoundingClientRect() : null;
    if (!rect) {
      try {
        const coords = editor.view.coordsAtPos(Math.min(pos, editor.state.doc.content.size));
        rect = new DOMRect(coords.left, coords.top, 0, coords.bottom - coords.top);
      } catch { /* no layout (tests): the popover takes a default place */ }
    }
    setMath({ kind, pos, latex, existing, rect, replace });
  }, []);

  const insertImages = useCallback(async (list: File[], at: number) => {
    const editor = editorRef.current;
    if (!editor) return;
    let pos = at;
    for (const file of list) {
      const refusal = assetRefusal(file);
      if (refusal) { latest.current.onError(refusal); continue; }
      setUploading(file.name || "image");
      try {
        const path = await uploadAsset(file);
        const node = { type: "image", attrs: { src: path, alt: altText(file.name) } };
        editor.chain().focus().insertContentAt(Math.min(pos, editor.state.doc.content.size), node).run();
        pos = editor.state.selection.to;
      } catch (error) {
        latest.current.onError(`Couldn't add ${file.name || "the image"}: ${error instanceof Error ? error.message : String(error)}`);
      } finally {
        setUploading(null);
      }
    }
  }, []);

  const host = useRef<BlockHost>({
    pickImage: at => { imageAt.current = at; files.current?.click(); },
    editMath: (kind, at) => openMath(kind, at, "", false),
    createPage: at => {
      void latest.current.onCreatePage(null).then(page => {
        const editor = editorRef.current;
        if (!page || !editor) return;
        editor.chain().focus().insertContentAt(Math.min(at, editor.state.doc.content.size), { type: "pageLink", attrs: page }).run();
      });
    },
    createDatabase: at => {
      void latest.current.onCreateDatabase?.().then(database => {
        const editor = editorRef.current;
        if (!database || !editor) return;
        editor.chain().focus().insertContentAt(Math.min(at, editor.state.doc.content.size), { type: "databaseEmbed", attrs: { id: database.id, view: null, title: database.title } }).run();
      });
    },
  });
  const wikiHost = () => ({
    pages: () => latest.current.pages.pages?.() ?? [],
    createByTitle: async (title: string) => {
      const page = await latest.current.onCreatePage(title);
      return page ? { id: page.id, title: page.title } : null;
    },
  });

  // Both menus share the keydown path; each answers only while its own menu
  // is open.
  const menuKeys = useCallback((event: KeyboardEvent) => {
    if (wikiMenu.current?.onKeyDown(event)) return true;
    return slashMenu.current?.onKeyDown(event) ?? false;
  }, []);

  const editor = useEditor({
    content: split.current.body,
    contentType: "markdown",
    extensions: noteExtensions({
      image: NoteImage,
      pageLink: NotePageLink,
      databaseEmbed: NoteDatabaseEmbed,
      wikiLink: NoteWikiLink,
      callout: NoteCallout,
      onMathClick: (kind, latex, pos) => openMath(kind, pos, latex, true),
      extra: [slashCommand(setSlash, menuKeys, () => host.current), wikiCommand(setWiki, menuKeys, wikiHost)],
    }),
    editorProps: {
      attributes: { class: "note-editor", "aria-label": "Your words", role: "textbox", "aria-multiline": "true", spellcheck: "true" },
      handlePaste: (view, event) => {
        const list = imageFiles(event.clipboardData);
        if (!list.length) return false;
        void insertImages(list, view.state.selection.from);
        return true;
      },
      handleDrop: (view, event, _slice, moved) => {
        const list = moved ? [] : imageFiles(event.dataTransfer);
        if (!list.length) return false;
        void insertImages(list, view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos ?? view.state.selection.from);
        return true;
      },
    },
    onCreate: ({ editor: created }) => {
      editorRef.current = created;
      installMinimalEscaping(created);
      codec.current = editorCodec(created);
    },
    onUpdate: () => {
      dirty.current = true;
      clearTimeout(timer.current);
      timer.current = setTimeout(commit, HAND_OVER_MS);
    },
    onBlur: () => { commit(); latest.current.onLeave?.(); },
    onDestroy: () => commit(),
  }, []);

  // A change from outside the editor, or a title change that alters whether a
  // leading `# Title` heading is hidden.
  useEffect(() => {
    if (!editor || !codec.current) return;
    if (props.text === emitted.current) {
      const current = split.current.head + (dirty.current ? markdown(editor) : source.current);
      const nextSplit = splitTitleHeading(current, props.title);
      if (nextSplit.head === split.current.head) return;
      split.current = nextSplit;
      source.current = nextSplit.body;
      const { from, to } = editor.state.selection;
      editor.commands.setContent(nextSplit.body, { contentType: "markdown", emitUpdate: false });
      const size = editor.state.doc.content.size;
      if (editor.isFocused) editor.commands.setTextSelection({ from: Math.min(from, size), to: Math.min(to, size) });
      return;
    }
    clearTimeout(timer.current);
    let next = props.text;
    if (dirty.current) {
      const mine = split.current.head + markdown(editor);
      next = rebaseDraft(emitted.current, mine, props.text).text;
      dirty.current = false;
    }
    split.current = splitTitleHeading(next, props.title);
    source.current = split.current.body;
    const { from, to } = editor.state.selection;
    editor.commands.setContent(split.current.body, { contentType: "markdown", emitUpdate: false });
    const size = editor.state.doc.content.size;
    if (editor.isFocused) editor.commands.setTextSelection({ from: Math.min(from, size), to: Math.min(to, size) });
    emitted.current = next;
    if (next !== props.text) latest.current.onChange(next);
  }, [editor, props.text, props.title, markdown]);

  useEffect(() => () => clearTimeout(timer.current), []);

  useImperativeHandle(ref, () => ({
    commit,
    focus: at => editor?.commands.focus(at ?? "end"),
  }), [commit, editor]);

  const saveMath = (latex: string) => {
    if (!editor || !math) return;
    const chain = editor.chain().focus();
    if (math.existing) {
      if (math.kind === "inline") chain.updateInlineMath({ latex, pos: math.pos });
      else chain.updateBlockMath({ latex, pos: math.pos });
    } else {
      if (math.replace) chain.deleteRange(math.replace);
      const pos = math.replace?.from ?? math.pos;
      if (math.kind === "inline") chain.insertInlineMath({ latex, pos });
      else chain.insertBlockMath({ latex, pos });
    }
    chain.run();
    setMath(null);
  };
  const removeMath = () => {
    if (!editor || !math) return;
    if (math.kind === "inline") editor.chain().focus().deleteInlineMath({ pos: math.pos }).run();
    else editor.chain().focus().deleteBlockMath({ pos: math.pos }).run();
    setMath(null);
  };

  return <EditorPagesContext.Provider value={props.pages}>
    <div ref={area} className="relative -ml-14 pl-14">
      {uploading && <p role="status" className="m-0 mb-2 text-[12.5px] text-muted-foreground">Adding {uploading}…</p>}
      <EditorContent editor={editor} />
      {editor && <BlockHandle editor={editor} area={area} />}
      {editor && <SelectionToolbar editor={editor} onMath={() => {
        const { from, to } = editor.state.selection;
        openMath("inline", from, editor.state.doc.textBetween(from, to), false, { from, to });
      }} />}
      {slash && <SlashMenu ref={slashMenu} state={slash} />}
      {wiki && <WikiMenu ref={wikiMenu} state={wiki} />}
      {math && <MathPopover key={`${math.pos}-${math.existing}`} edit={math} onSave={saveMath} onRemove={removeMath} onClose={() => { setMath(null); editor?.commands.focus(); }} />}
      <input
        ref={files}
        type="file"
        accept={ASSET_TYPES.join(",")}
        multiple
        hidden
        aria-hidden="true"
        tabIndex={-1}
        onChange={event => {
          const list = Array.from(event.target.files ?? []);
          event.target.value = "";
          if (list.length && editor) void insertImages(list, imageAt.current ?? editor.state.selection.from);
        }}
      />
    </div>
  </EditorPagesContext.Provider>;
});
