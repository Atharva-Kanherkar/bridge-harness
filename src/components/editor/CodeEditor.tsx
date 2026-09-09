import { useEffect, useRef } from "react";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, rectangularSelection, crosshairCursor, highlightSpecialChars } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, foldGutter, foldKeymap, indentOnInput, indentUnit, syntaxHighlighting } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { languageFromPath } from "../highlight";
import { bridgeHighlighter } from "./highlighter";
import { loadLanguage } from "./language";

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
export function CodeEditor({ docKey, doc, path, readOnly = false, visible = true, revealLine, onChange, onSave, className }: {
  docKey: string;
  doc: string;
  path: string;
  readOnly?: boolean;
  /** False while the editor is display:none — it must re-measure on return. */
  visible?: boolean;
  /** Move the cursor to a line and scroll it into view — once per nonce, so a
   *  re-render does not yank the caret back after the user moves on. */
  revealLine?: { line: number; nonce: number };
  onChange: (value: string) => void;
  onSave: () => void;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView>();
  // Callbacks live in refs so a re-render never rebuilds the editor.
  const handlers = useRef({ onChange, onSave });
  handlers.current = { onChange, onSave };

  useEffect(() => {
    if (!host.current) return;
    const language = new Compartment();
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc,
        extensions: [
          ...baseExtensions(() => handlers.current.onSave()),
          language.of([]),
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
          EditorView.updateListener.of(update => {
            if (update.docChanged) handlers.current.onChange(update.state.doc.toString());
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
