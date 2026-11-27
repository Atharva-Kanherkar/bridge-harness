import { useEffect, useRef } from "react";
import { Compartment, EditorState, Transaction, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, rectangularSelection, crosshairCursor, highlightSpecialChars } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, foldGutter, foldKeymap, indentOnInput, indentUnit, syntaxHighlighting } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { languageFromPath } from "../highlight";
import { bridgeHighlighter } from "./highlighter";
import { loadLanguage } from "./language";
import { useLatest } from "../../useLatest";

/**
 * The editor's fixed extension set.
 *
 * `bridgeHighlighter` is the load-bearing choice: it emits `stx-*` class
 * names instead of inline styles, so the whole editor is themed from
 * `index.css` with the same `--syn-*` tokens — and the same class names — as
 * the diff viewer and markdown code blocks. No second palette, no second
 * class vocabulary, and no CSS-in-JS. See `highlighter.ts` for why
 * `@lezer/highlight`'s stock `classHighlighter` is not that.
 */
function baseExtensions(onSave: () => void): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    history(),
    foldGutter(),
    drawSelection(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    indentUnit.of("  "),
    syntaxHighlighting(bridgeHighlighter),
    bracketMatching(),
    closeBrackets(),
    rectangularSelection(),
    crosshairCursor(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    search({ top: true }),
    EditorView.lineWrapping,
    keymap.of([
      // Save comes first so ⌘S never falls through to the browser.
      { key: "Mod-s", preventDefault: true, run: () => (onSave(), true) },
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      indentWithTab,
    ]),
  ];
}

/**
 * A CodeMirror 6 document bound to one file.
 *
 * Deliberately not a controlled component: re-creating the state on every
 * keystroke would throw away the cursor, the undo history, and the fold state.
 * `doc` seeds the document, and later changes flow out through `onChange`.
 * Changing `docKey` (the file being edited) is what re-seeds it.
 */
/**
 * The smallest single replacement that turns `before` into `after`: the
 * common prefix and suffix stay put. Replacing the whole document instead
 * maps every selection to one end of it, so an agent's one-line write threw
 * the caret to the top of the file.
 */
export function minimalReplacement(before: string, after: string): { from: number; to: number; insert: string } | null {
  if (before === after) return null;
  let start = 0;
  const shortest = Math.min(before.length, after.length);
  while (start < shortest && before.charCodeAt(start) === after.charCodeAt(start)) start += 1;
  let endBefore = before.length;
  let endAfter = after.length;
  while (endBefore > start && endAfter > start && before.charCodeAt(endBefore - 1) === after.charCodeAt(endAfter - 1)) {
    endBefore -= 1;
    endAfter -= 1;
  }
  return { from: start, to: endBefore, insert: after.slice(start, endAfter) };
}

/**
 * The file's own line break, when it has exactly one kind that is not "\n".
 * CodeMirror reads every break style but writes "\n", so a CRLF file saved
 * from the editor came back with every line changed, and an edit undone
 * never read as clean again. Mixed files keep the default.
 */
export function lineSeparatorOf(text: string): "\r\n" | undefined {
  if (!text.includes("\r\n")) return undefined;
  return text.replace(/\r\n/g, "").search(/[\r\n]/) === -1 ? "\r\n" : undefined;
}

export function CodeEditor({ docKey, doc, path, readOnly = false, visible = true, revealLine, syncDoc, onChange, onSave, className }: {
  docKey: string;
  doc: string;
  path: string;
  readOnly?: boolean;
  /** False while the editor is display:none — it must re-measure on return. */
  visible?: boolean;
  /** Move the cursor to a line and scroll it into view — once per nonce, so a
   *  re-render does not yank the caret back after the user moves on. */
  revealLine?: { line: number; nonce: number };
  /** Replace the document in place, once per version, keeping the editor,
   *  its caret, scroll, and undo history. This is how bytes an agent wrote
   *  reach a clean buffer without remounting it. */
  syncDoc?: { text: string; version: number };
  onChange: (value: string) => void;
  onSave: () => void;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>();
  // Callbacks live in refs so a re-render never rebuilds the editor.
  const handlers = useLatest({ onChange, onSave });

  const syncSeen = useRef(syncDoc?.version ?? 0);
  const separator = useRef<string | undefined>(undefined);

  useEffect(() => {
    if (!host.current) return;
    syncSeen.current = syncDoc?.version ?? 0;
    separator.current = lineSeparatorOf(doc);
    const language = new Compartment();
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc,
        extensions: [
          ...baseExtensions(() => handlers.current.onSave()),
          ...(separator.current ? [EditorState.lineSeparator.of(separator.current)] : []),
          language.of([]),
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
          EditorView.updateListener.of(update => {
            // `sliceDoc` joins with the file's own separator; `doc.toString()`
            // always uses "\n".
            if (update.docChanged) handlers.current.onChange(update.state.sliceDoc());
          }),
        ],
      }),
    });
    view.current = editor;
    let live = true;
    // Grammars load after first paint: the file is on screen and typeable
    // immediately, and colour arrives a frame or two later.
    void loadLanguage(languageFromPath(path)).then(support => {
      if (live && support) editor.dispatch({ effects: language.reconfigure(support) });
    });
    return () => {
      live = false;
      editor.destroy();
      view.current = undefined;
    };
    // docKey identifies the file; doc/path/readOnly are seeds read at creation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [docKey]);

  // A hidden editor measures as zero-height; ask for a fresh measurement when
  // its tab comes back, or the first scroll lands in the wrong place.
  useEffect(() => {
    if (visible) view.current?.requestMeasure();
  }, [visible]);

  useEffect(() => {
    const editor = view.current;
    if (!editor || !syncDoc || syncDoc.version === syncSeen.current) return;
    syncSeen.current = syncDoc.version;
    // Diffed in document space, where every line break is one position.
    const change = minimalReplacement(editor.state.doc.toString(), syncDoc.text.replace(/\r\n?/g, "\n"));
    if (!change) return;
    const insert = separator.current ? change.insert.replace(/\n/g, separator.current) : change.insert;
    // Out of history: ⌘Z undoes what *you* typed, not the agent's write.
    editor.dispatch({ changes: { ...change, insert }, annotations: Transaction.addToHistory.of(false) });
  }, [syncDoc]);

  const revealSeen = useRef(0);
  useEffect(() => {
    const editor = view.current;
    if (!editor || !revealLine || revealLine.nonce === revealSeen.current) return;
    revealSeen.current = revealLine.nonce;
    const line = Math.min(Math.max(revealLine.line, 1), editor.state.doc.lines);
    const position = editor.state.doc.line(line).from;
    editor.dispatch({
      selection: { anchor: position },
      effects: EditorView.scrollIntoView(position, { y: "center" }),
    });
  }, [revealLine]);

  return <div ref={host} className={className} />;
}
