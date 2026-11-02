// @vitest-environment jsdom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Editor } from "@tiptap/core";
import { resetMockNotes } from "../../../notesMock";
import { NoteEditor, HAND_OVER_MS, type NoteEditorHandle, type NoteEditorProps } from "./NoteEditor";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect ??= () => new DOMRect(0, 0, 0, 0);
  resetMockNotes();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const settle = async (ms = 0) => { await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); }); };
const handle = createRef<NoteEditorHandle>();

async function mount(text: string, overrides: Partial<NoteEditorProps> = {}) {
  const onChange = vi.fn();
  const props: NoteEditorProps = {
    text, title: null, onChange,
    pages: {
      title: () => undefined,
      open: vi.fn(),
      openByTitle: vi.fn(),
      pages: () => [{ id: "p-plans", title: "Plans" }, { id: "p-crew", title: "Crew notes" }],
    },
    onCreatePage: vi.fn(async (title: string | null) => ({ id: "child-1", title: title ?? "Child" })),
    onError: vi.fn(),
    ...overrides,
  };
  await act(async () => { root.render(<NoteEditor ref={handle} {...props} />); });
  await settle();
  return { props, onChange, rerender: async (next: Partial<NoteEditorProps>) => { await act(async () => { root.render(<NoteEditor ref={handle} {...props} {...next} />); }); await settle(); } };
}
const editor = () => (host.querySelector(".note-editor") as HTMLElement & { editor: Editor }).editor;
const commit = async () => { await act(async () => { handle.current!.commit(); }); };
const key = async (name: string) => { await act(async () => { editor().view.dom.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true })); }); await settle(); };
/** Type on a new line at the end, where `/` opens the block menu. */
const typeOnNewLine = async (text: string) => {
  await act(async () => { editor().chain().focus("end").insertContent({ type: "paragraph" }).run(); });
  await act(async () => { editor().commands.insertContent(text); });
  await settle();
};
/** Append text to the paragraph that holds `needle`. */
const typeAfter = async (needle: string, text: string) => {
  let at = -1;
  editor().state.doc.descendants((node, pos) => { if (at < 0 && node.isText && node.text!.includes(needle)) at = pos + node.text!.indexOf(needle) + needle.length; });
  if (at < 0) throw new Error(`no ${needle}`);
  await act(async () => { editor().chain().setTextSelection(at).insertContent(text).run(); });
};

const EVERY_BLOCK = [
  "# Heading one",
  "## Heading two",
  "### Heading three",
  "A paragraph with **bold**, *soft*, ~~gone~~, `code`, <u>under</u>, a [link](https://example.com) and $x^2$ inline.",
  "- bullet\n- another\n  - nested",
  "1. first\n2. second",
  "- [ ] open\n- [x] done",
  "> a quote",
  "> [!WARNING]\n> Careful here",
  "<details>\n<summary>More</summary>\n\nHidden words\n\n</details>",
  "```ts\nconst x = 1;\n```",
  "| a | b |\n| --- | --- |\n| 1 | 2 |",
  "---",
  "![A diagram](assets/0123456789abcdef.png)",
  "$$\n\\int_0^1 x\\,dx\n$$",
  "<page url=\"bridge://p/0190-child\">Child page</page>",
  "<database id=\"d1\"></database>",
].join("\n\n") + "\n";

describe("NoteEditor", () => {
  it("draws every block and hands nothing over when nothing was typed", async () => {
    const { onChange } = await mount(EVERY_BLOCK);
    const dom = host.querySelector(".note-editor")!;
    for (const selector of ["h1", "h2", "h3", "strong", "em", "s", "code", "u", "a", "[data-type='inline-math']", "ul li", "ol li", "ul[data-type='taskList']", "blockquote", "aside[data-callout][data-kind='WARNING']", "[data-type='details']", "pre", "table", "hr", "figure.note-image", "[data-type='block-math']", "[data-raw-block]"]) {
      expect(dom.querySelector(selector), selector).not.toBeNull();
    }
    expect(dom.textContent).toContain("Child page");
    await commit();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("changes only the block that was typed in, for every block form", async () => {
    const { onChange } = await mount(EVERY_BLOCK);
    await typeAfter("Heading two", " and more");
    await commit();
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0]).toBe(EVERY_BLOCK.replace("## Heading two", "## Heading two and more"));
  });

  it("keeps a 200-block page byte for byte outside the block typed in", async () => {
    const blocks = Array.from({ length: 200 }, (_, index) => index % 4 === 0 ? `## Part ${index}` : index % 4 === 1 ? `* item ${index}\n* item ${index}b` : `Paragraph ${index}   with  __old__ style.`);
    const text = `${blocks.join("\n\n")}\n`;
    const { onChange } = await mount(text);
    await typeAfter("Paragraph 102", "!");
    await commit();
    expect(onChange.mock.calls[0][0]).toBe(text.replace("Paragraph 102   with  __old__ style.", "Paragraph 102!   with  **old** style."));
  });

  it("hands typing over after a pause, or at once on commit", async () => {
    const { onChange } = await mount("one\n");
    await typeAfter("one", " two");
    expect(onChange).not.toHaveBeenCalled();
    await settle(HAND_OVER_MS + 50);
    expect(onChange).toHaveBeenLastCalledWith("one two\n");
  });

  it("opens the / menu, filters it, inserts a Heading 2 on Enter and closes on Escape", async () => {
    await mount("first\n");
    await act(async () => { editor().chain().setTextSelection(1).insertContent("/").run(); });
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Blocks']")).not.toBeNull();
    await typeAfter("/", "h2");
    await settle();
    const options = [...document.querySelectorAll("[role='listbox'][aria-label='Blocks'] [role='option']")].map(node => node.textContent);
    expect(options).toEqual([expect.stringContaining("Heading 2")]);
    await key("Enter");
    expect(editor().state.doc.firstChild?.type.name).toBe("heading");
    expect(editor().state.doc.firstChild?.attrs.level).toBe(2);
    expect(editor().state.doc.textContent).toBe("first");
    expect(document.querySelector("[role='listbox'][aria-label='Blocks']")).toBeNull();

    await typeAfter("first", " /");
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Blocks']")).not.toBeNull();
    await key("Escape");
    expect(document.querySelector("[role='listbox'][aria-label='Blocks']")).toBeNull();
  });

  it("adds a child page from the / menu", async () => {
    const onCreatePage = vi.fn(async () => ({ id: "0190-new", title: "Untitled" }));
    const { onChange } = await mount("Intro\n", { onCreatePage });
    await typeOnNewLine("/page");
    await key("Enter");
    await settle();
    expect(onCreatePage).toHaveBeenCalledTimes(1);
    await commit();
    expect(onChange.mock.calls.at(-1)?.[0]).toBe("Intro\n\n<page url=\"bridge://p/0190-new\">Untitled</page>\n");
  });

  it("writes a math block from the / menu through the formula editor", async () => {
    const { onChange } = await mount("Intro\n");
    await typeOnNewLine("/math");
    await key("Enter");
    const tex = document.querySelector<HTMLTextAreaElement>("textarea[aria-label='TeX']")!;
    expect(tex).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(tex, "e^{i\\pi} + 1 = 0");
      tex.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.querySelector("form[aria-label='Math block']")?.textContent).not.toContain("parse error");
    await act(async () => { tex.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    await settle();
    await commit();
    expect(onChange.mock.calls.at(-1)?.[0]).toContain("$$\ne^{i\\pi} + 1 = 0\n$$");
  });

  it("uploads a picked image and writes it as an asset link", async () => {
    const onError = vi.fn();
    const { onChange } = await mount("Intro\n", { onError });
    await typeOnNewLine("/image");
    const input = host.querySelector<HTMLInputElement>("input[type='file']")!;
    const click = vi.spyOn(input, "click").mockImplementation(() => undefined);
    await key("Enter");
    expect(click).toHaveBeenCalled();
    const file = new File([new Uint8Array([137, 80, 78, 71, 1, 2, 3])], "diagram.png", { type: "image/png" });
    Object.defineProperty(input, "files", { value: [file], configurable: true });
    await act(async () => { input.dispatchEvent(new Event("change", { bubbles: true })); });
    await settle(20);
    await commit();
    expect(onError).not.toHaveBeenCalled();
    expect(onChange.mock.calls.at(-1)?.[0]).toMatch(/!\[diagram\]\(assets\/[0-9a-f]{16}\.png\)/);
    expect(host.querySelector("figure img")?.getAttribute("src")).toMatch(/^blob:|^data:/);
  });

  it("refuses a file that is not an image it can store", async () => {
    const onError = vi.fn();
    await mount("Intro\n", { onError });
    await typeOnNewLine("/image");
    const input = host.querySelector<HTMLInputElement>("input[type='file']")!;
    vi.spyOn(input, "click").mockImplementation(() => undefined);
    await key("Enter");
    Object.defineProperty(input, "files", { value: [new File(["x"], "notes.pdf", { type: "application/pdf" })], configurable: true });
    await act(async () => { input.dispatchEvent(new Event("change", { bubbles: true })); });
    await settle();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("not an image"));
  });

  it("takes an outside change as is when nothing is pending, and merges it with pending typing", async () => {
    const { rerender, onChange } = await mount("alpha\n\nbeta\n");
    await rerender({ text: "alpha\n\nbeta\n\nagent line\n" });
    expect(editor().getText()).toContain("agent line");
    expect(onChange).not.toHaveBeenCalled();

    await typeAfter("alpha", " mine");
    await rerender({ text: "alpha\n\nbeta\n\nagent line\n\nagent two\n" });
    expect(editor().getText()).toContain("alpha mine");
    expect(editor().getText()).toContain("agent two");
    expect(onChange).toHaveBeenLastCalledWith("alpha mine\n\nbeta\n\nagent line\n\nagent two\n");
  });

  it("keeps a heading that repeats the title out of the editor, and in the text", async () => {
    const { rerender, onChange } = await mount("# Launch\n\nBody\n", { title: "Launch" });
    expect(host.querySelector(".note-editor h1")).toBeNull();
    await typeAfter("Body", " more");
    await commit();
    expect(onChange).toHaveBeenLastCalledWith("# Launch\n\nBody more\n");

    await rerender({ text: "# Launch\n\nBody more\n", title: "Renamed" });
    expect(host.querySelector(".note-editor h1")?.textContent).toBe("Launch");
    await typeAfter("Body more", "!");
    await commit();
    expect(onChange).toHaveBeenLastCalledWith("# Launch\n\nBody more!\n");
  });

  it("keeps untouched blocks with CRLF line endings and open details attributes", async () => {
    const crlf = "First  with __bold__.\r\n\r\n<details open>\r\n<summary>More</summary>\r\n\r\nHidden\r\n\r\n</details>\r\n\r\nLast line\r\n";
    const { onChange } = await mount(crlf);
    await typeAfter("Last line", "!");
    await commit();
    expect(onChange).toHaveBeenLastCalledWith("First  with __bold__.\r\n\r\n<details open>\r\n<summary>More</summary>\r\n\r\nHidden\r\n\r\n</details>\r\n\r\nLast line!\r\n");
  });

  it("round-trips wiki links, with and without an alias, beside untouched blocks", async () => {
    const before = "Before\n\nSee [[Plans]] then [[Crew|the crew]].\n\nAfter\n";
    const { onChange } = await mount(before);
    const dom = host.querySelector(".note-editor")!;
    expect([...dom.querySelectorAll("button[aria-label^='Open the page']")].map(node => node.textContent)).toEqual(["Plans", "the crew"]);
    await commit();
    expect(onChange).not.toHaveBeenCalled();
    await typeAfter("Before", "!");
    await commit();
    expect(onChange.mock.calls[0][0]).toBe(before.replace("Before", "Before!"));
  });

  it("escapes brackets only where a wiki link could be misread", async () => {
    const { onChange } = await mount("plain [1] stays\n");
    await typeAfter("stays", " typed [[maybe]] else");
    await commit();
    expect(onChange.mock.calls[0][0]).toBe("plain \\[1] stays typed \\[\\[maybe]] else\n");
  });

  it("opens the [[ menu on typing, filters pages, and inserts a link on Enter", async () => {
    await mount("first\n");
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[").run(); });
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).not.toBeNull();
    await typeAfter("[[", "pl");
    await settle();
    const options = [...document.querySelectorAll("[role='listbox'][aria-label='Page links'] [role='option']")].map(node => node.textContent);
    expect(options).toEqual(["Plans", 'Make "pl"']);
    await key("Enter");
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).toBeNull();
    expect(editor().getMarkdown()).toContain("[[Plans]]");
  });

  it("closes the [[ menu on Escape without inserting anything", async () => {
    await mount("first\n");
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[Crew").run(); });
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).not.toBeNull();
    await key("Escape");
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).toBeNull();
    expect(editor().getMarkdown()).toContain("[[Crew");
  });

  it("offers to make the page when no existing title matches, and links to it", async () => {
    const { props: given } = await mount("first\n");
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[Fresh thoughts").run(); });
    await settle();
    const options = [...document.querySelectorAll("[role='listbox'][aria-label='Page links'] [role='option']")].map(node => node.textContent);
    expect(options).toEqual(['Make "Fresh thoughts"']);
    await key("Enter");
    await settle();
    expect(given.onCreatePage).toHaveBeenCalledWith("Fresh thoughts");
    expect(editor().getMarkdown()).toContain("[[Fresh thoughts]]");
  });

  it("opens the page named in a wiki link when its pill is clicked", async () => {
    const { props: given } = await mount("See [[Plans]] and continue\n");
    const pill = [...host.querySelectorAll("button[aria-label]")].find(node => node.getAttribute("aria-label") === "Open the page Plans");
    expect(pill).not.toBeNull();
    await act(async () => { pill!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(given.pages.openByTitle).toHaveBeenCalledWith("Plans");
  });
});
