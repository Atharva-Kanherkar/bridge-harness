// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { CodeEditor, lineSeparatorOf, minimalReplacement } from "./CodeEditor";

// The real CodeMirror, not a stand-in: line separators and in-place sync are
// properties of the editor state, and a textarea mock cannot show them.

beforeAll(() => {
  // jsdom has no layout; CodeMirror only needs these to exist.
  const rects = () => Object.assign([], { item: () => null }) as unknown as DOMRectList;
  Range.prototype.getClientRects ??= rects;
  Range.prototype.getBoundingClientRect ??= () => new DOMRect();
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

const view = () => EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!;

describe("lineSeparatorOf", () => {
  it("keeps a pure CRLF file's separator and defaults everything else", () => {
    expect(lineSeparatorOf("a\r\nb\r\n")).toBe("\r\n");
    expect(lineSeparatorOf("a\nb\n")).toBeUndefined();
    expect(lineSeparatorOf("a\r\nb\n")).toBeUndefined();
    expect(lineSeparatorOf("")).toBeUndefined();
  });
});

describe("minimalReplacement", () => {
  it("replaces only the span between the common prefix and suffix", () => {
    expect(minimalReplacement("one\ntwo\nthree", "one\nTWO\nthree")).toEqual({ from: 4, to: 7, insert: "TWO" });
    expect(minimalReplacement("abc", "abXc")).toEqual({ from: 2, to: 2, insert: "X" });
    expect(minimalReplacement("aaa", "aa")).toEqual({ from: 2, to: 3, insert: "" });
    expect(minimalReplacement("same", "same")).toBeNull();
  });
});

describe("CodeEditor", () => {
  it("reports edits to a CRLF file with CRLF line breaks", async () => {
    const onChange = vi.fn();
    await act(async () => { root.render(<CodeEditor docKey="k" doc={"a\r\nb\r\n"} path="a.txt" onChange={onChange} onSave={() => undefined} />); });
    await act(async () => { view().dispatch({ changes: { from: 1, insert: "X" } }); });
    expect(onChange).toHaveBeenLastCalledWith("aX\r\nb\r\n");
  });

  it("reports an LF file unchanged in style", async () => {
    const onChange = vi.fn();
    await act(async () => { root.render(<CodeEditor docKey="k" doc={"a\nb\n"} path="a.txt" onChange={onChange} onSave={() => undefined} />); });
    await act(async () => { view().dispatch({ changes: { from: 1, insert: "X" } }); });
    expect(onChange).toHaveBeenLastCalledWith("aX\nb\n");
  });

  it("applies a sync in place and keeps the caret where it was", async () => {
    const onChange = vi.fn();
    const props = { docKey: "k", doc: "one\ntwo\nthree\n", path: "a.txt", onChange, onSave: () => undefined };
    await act(async () => { root.render(<CodeEditor {...props} />); });
    const before = view();
    const caret = before.state.doc.line(3).from + 2;
    await act(async () => { before.dispatch({ selection: { anchor: caret } }); });

    await act(async () => { root.render(<CodeEditor {...props} syncDoc={{ text: "one\nTWO!\nthree\n", version: 1 }} />); });
    expect(view()).toBe(before);
    expect(before.state.doc.toString()).toBe("one\nTWO!\nthree\n");
    expect(before.state.sliceDoc(before.state.selection.main.head - 2, before.state.selection.main.head + 3)).toBe("three");
    expect(onChange).toHaveBeenLastCalledWith("one\nTWO!\nthree\n");
  });

  it("syncs a CRLF file without breaking its line endings", async () => {
    const onChange = vi.fn();
    const props = { docKey: "k", doc: "a\r\nb\r\n", path: "a.txt", onChange, onSave: () => undefined };
    await act(async () => { root.render(<CodeEditor {...props} />); });
    await act(async () => { root.render(<CodeEditor {...props} syncDoc={{ text: "a\r\nB\r\nc\r\n", version: 1 }} />); });
    expect(onChange).toHaveBeenLastCalledWith("a\r\nB\r\nc\r\n");
    expect(view().state.doc.lines).toBe(4);
  });

  it("applies each sync version once", async () => {
    const onChange = vi.fn();
    const props = { docKey: "k", doc: "x", path: "a.txt", onChange, onSave: () => undefined };
    const sync = { text: "y", version: 1 };
    await act(async () => { root.render(<CodeEditor {...props} syncDoc={sync} />); });
    // Mounted with version 1 already: the seed is the doc, not the sync.
    expect(view().state.doc.toString()).toBe("x");
    await act(async () => { view().dispatch({ changes: { from: 0, to: 1, insert: "typed" } }); });
    await act(async () => { root.render(<CodeEditor {...props} syncDoc={{ ...sync }} />); });
    expect(view().state.doc.toString()).toBe("typed");
  });
});
