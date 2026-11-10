// @vitest-environment jsdom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Editor } from "@tiptap/core";
import { resetMockNotes } from "../../../notesMock";
import { NoteEditor, HAND_OVER_MS, type NoteEditorHandle, type NoteEditorProps } from "./NoteEditor";
import { imageWidthOf } from "./views";
import { BLOCK_CHOICES } from "./slash";

// No core assets in jsdom: images draw from a fake url, which still exercises
// the caption and the resize handles.
// Excalidraw itself needs a real canvas; the block around it is what is tested here.
vi.mock("./drawingCanvas", () => ({
  DrawingPreview: ({ scene }: { scene: { elements: unknown[] } }) => <p data-preview="">{scene.elements.length} elements</p>,
  DrawingEditor: ({ body, onSave, onClose }: { body: string; onSave: (body: string) => void; onClose: () => void }) => <div data-drawing-editor="">
    <button type="button" onClick={() => { onSave(body.replace('"elements":[\n', '"elements":[\n{"id":"n","type":"rectangle","x":0,"y":0,"width":10,"height":10,"version":1},\n')); onClose(); }}>Save drawing</button>
  </div>,
}));
vi.mock("./assets", async actual => ({ ...(await actual<typeof import("./assets")>()), assetUrl: async (source: string) => `blob:asset${source}` }));

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
  "<database url=\"bridge://p/0190-db\" view=\"v-board\">Reading &lt;list&gt;</database>",
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

  it("a database block round-trips byte for byte beside every other block", async () => {
    const renderDatabase = vi.fn((id: string, view: string | null) => <p data-testid="table">{id} {view}</p>);
    const open = vi.fn();
    const { onChange } = await mount(EVERY_BLOCK, { pages: { title: () => undefined, open, renderDatabase } });
    expect(renderDatabase).toHaveBeenCalledWith("0190-db", "v-board");
    expect(host.querySelector("[data-database-frame] [data-testid=table]")?.textContent).toBe("0190-db v-board");
    expect(host.textContent).toContain("Reading <list>");
    await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === "Open as page")!.click(); });
    expect(open).toHaveBeenCalledWith("0190-db");
    await commit();
    expect(onChange).not.toHaveBeenCalled();
    // Typing elsewhere keeps the block exactly as written.
    await typeAfter("Heading two", "!");
    await commit();
    expect(onChange.mock.calls[0][0]).toBe(EVERY_BLOCK.replace("## Heading two", "## Heading two!"));
    // Written fresh, the block takes the same form.
    expect(editor().getMarkdown()).toContain("<database url=\"bridge://p/0190-db\" view=\"v-board\">Reading &lt;list&gt;</database>");
  });

  it("a database tag with other attributes stays raw", async () => {
    const forms = ["<database id=\"d1\"></database>", "<database url=\"bridge://p/x\" extra=\"1\">T</database>", "<database url=\"bridge://p/x\">\nT\n</database>"];
    for (const form of forms) {
      await mount(`Intro\n\n${form}\n`);
      expect(editor().state.doc.child(1).type.name, form).toBe("rawBlock");
      expect(editor().state.doc.child(1).attrs.source, form).toBe(form);
      act(() => root.render(<></>));
    }
    await mount("Intro\n\n<database url=\"bridge://p/x\">T</database>\n");
    expect(editor().state.doc.child(1).type.name).toBe("databaseEmbed");
    expect(editor().state.doc.child(1).attrs).toEqual({ id: "x", view: null, title: "T" });
  });

  it("the / menu's Database makes a database under the page and inserts its block", async () => {
    const onCreateDatabase = vi.fn(async () => ({ id: "0190-db", title: "Untitled database" }));
    const { onChange } = await mount("Intro\n", { onCreateDatabase });
    await typeOnNewLine("/database");
    const options = [...document.querySelectorAll("[role='listbox'][aria-label='Blocks'] [role='option']")].map(node => node.textContent);
    expect(options).toEqual([expect.stringContaining("Database")]);
    await key("Enter");
    await settle();
    expect(onCreateDatabase).toHaveBeenCalledTimes(1);
    await commit();
    expect(onChange.mock.calls.at(-1)?.[0]).toBe("Intro\n\n<database url=\"bridge://p/0190-db\">Untitled database</database>\n");
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

  it("keeps a wiki link's spacing, empty alias and inner bracket byte for byte, as the core reads them", async () => {
    const before = "Lead\n\n[[Foo | Bar]] and [[Foo|]] and [[a[b]] here.\n";
    const { onChange } = await mount(before);
    expect([...host.querySelectorAll("button[aria-label^='Open the page']")].map(node => node.textContent)).toEqual(["Bar", "Foo", "a[b"]);
    await typeAfter("here", "!");
    await commit();
    expect(onChange.mock.calls[0][0]).toBe(before.replace("here.", "here!."));
  });

  it("turns a [[link]] typed straight through into a link", async () => {
    await mount("first\n");
    const view = editor().view;
    const type = (text: string) => {
      for (const char of text) {
        const { from, to } = view.state.selection;
        const handled = view.someProp("handleTextInput", handler => handler(view, from, to, char, () => view.state.tr.insertText(char, from, to)));
        if (!handled) view.dispatch(view.state.tr.insertText(char, from, to));
      }
    };
    await act(async () => { editor().commands.setTextSelection(6); type(" [[Meeting notes]]"); });
    await settle();
    expect(editor().getMarkdown()).toContain("first [[Meeting notes]]");
    expect(host.querySelector("button[aria-label='Open the page Meeting notes']")).not.toBeNull();
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

  it("opens no [[ menu inside inline code, and shows at most 20 pages", async () => {
    const many = Array.from({ length: 30 }, (_, index) => ({ id: `p-${index}`, title: `Plan ${index}` }));
    await mount("say `code` here\n", { pages: { title: () => undefined, open: vi.fn(), pages: () => many } });
    await typeAfter("cod", "[[Pl");
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).toBeNull();
    await typeAfter("here", " [[Plan");
    await settle();
    expect(document.querySelectorAll("[role='listbox'][aria-label='Page links'] [role='option']")).toHaveLength(21);
  });

  it("lets go of the [[ menu once the link is closed by hand", async () => {
    await mount("first\n");
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[Plans").run(); });
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).not.toBeNull();
    await act(async () => { editor().chain().insertContent("]] and more").run(); });
    await settle();
    expect(document.querySelector("[role='listbox'][aria-label='Page links']")).toBeNull();
  });

  it("links a page whose title [[ ]] cannot hold by its id", async () => {
    await mount("first\n", { pages: { title: () => undefined, open: vi.fn(), pages: () => [{ id: "p-q4", title: "Q4 | Launches" }] } });
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[Q4").run(); });
    await settle();
    await key("Enter");
    await settle();
    expect(editor().getMarkdown()).toContain('<page url="bridge://p/p-q4">Q4 | Launches</page>');
    expect(editor().getMarkdown()).not.toContain("[[Q4");
  });

  it("opens the page named in a wiki link when its pill is clicked", async () => {
    const { props: given } = await mount("See [[Plans]] and continue\n");
    const pill = [...host.querySelectorAll("button[aria-label]")].find(node => node.getAttribute("aria-label") === "Open the page Plans");
    expect(pill).not.toBeNull();
    await act(async () => { pill!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(given.pages.openByTitle).toHaveBeenCalledWith("Plans");
  });

  it("edits an image caption and round-trips a set width through its Markdown", async () => {
    const { onChange } = await mount("Before\n\n![Old plan](assets/0123456789abcdef.png \"420\")\n\n![](assets/89abcdef01234567.png)\n");
    const figure = host.querySelector("figure.note-image")!;
    const caption = figure.querySelector("figcaption[aria-label='Caption']")!;
    expect(caption.textContent).toBe("Old plan");
    expect((figure.querySelector("img") as HTMLElement).style.width).toBe("420px");
    expect([...host.querySelectorAll("figcaption[aria-label='Caption']")].filter(node => node.getAttribute("data-empty") === null).length).toBe(1);

    // The caption is edited like text: retype it, leave, and only that image
    // block's Markdown changes.
    await act(async () => {
      caption.textContent = "Old plan, revised";
      caption.dispatchEvent(new Event("input", { bubbles: true }));
      caption.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });
    await settle();
    await commit();
    expect(onChange.mock.calls[0][0]).toBe("Before\n\n![Old plan, revised](assets/0123456789abcdef.png \"420\")\n\n![](assets/89abcdef01234567.png)\n");

    // A bracket would end `![…]`, so a typed one is left out.
    await act(async () => {
      caption.textContent = "step 2] [final";
      caption.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });
    await settle();
    await commit();
    expect(onChange.mock.lastCall![0]).toContain("![step 2 final](assets/0123456789abcdef.png \"420\")");
    expect(caption.textContent).toBe("step 2 final");
    expect(imageWidthOf("0")).toBeNull();
  });

  it("shows an empty caption while its image is selected, and keeps a real title", async () => {
    await mount("Intro\n\n![](assets/0123456789abcdef.png)\n\n![](assets/89abcdef01234567.png \"Figure 1\")\n");
    const [plain, titled] = [...host.querySelectorAll("figure.note-image")];
    expect(plain.querySelector("figcaption")!.className).toContain("data-[empty]:hidden");
    await act(async () => { editor().commands.setNodeSelection(editor().state.doc.child(0).nodeSize); });
    expect(plain.querySelector("figcaption")!.className).not.toContain("data-[empty]:hidden");
    // A worded title is the image's own: shown, never overwritten by a drag.
    expect(titled.querySelector("img")!.getAttribute("title")).toBe("Figure 1");
    expect(titled.querySelector("button[aria-label^='Resize']")).toBeNull();
  });

  it("keeps the handles on an image whose width reads 0, drawn at its natural size", async () => {
    await mount("Intro\n\n![](assets/0123456789abcdef.png \"0\")\n");
    const image = host.querySelector("figure.note-image img") as HTMLElement;
    expect(image.style.width).toBe("");
    expect(image.getAttribute("title")).toBeNull();
    expect(host.querySelector("button[aria-label='Resize from the right edge']")).not.toBeNull();
  });

  it("links a page that shares an older page's title by its id", async () => {
    await mount("first\n", { pages: { title: () => undefined, open: vi.fn(), pages: () => [{ id: "p-new", title: "Notes", byId: true }, { id: "p-old", title: "Notes" }] } });
    await act(async () => { editor().chain().setTextSelection(1).insertContent("[[Notes").run(); });
    await settle();
    await key("Enter");
    await settle();
    expect(editor().getMarkdown()).toContain('<page url="bridge://p/p-new">Notes</page>');
  });

  it("selects only the caption on select all inside it", async () => {
    await mount("Intro words\n\n![Old plan](assets/0123456789abcdef.png)\n");
    const caption = host.querySelector<HTMLElement>("figcaption[aria-label='Caption']")!;
    await act(async () => { caption.dispatchEvent(new KeyboardEvent("keydown", { key: "a", metaKey: true, bubbles: true, cancelable: true })); });
    expect(getSelection()?.toString()).toBe("Old plan");
  });

  it("resizes an image from its handles, clamping to at least 120px", async () => {
    const { onChange } = await mount("Intro\n\n![Plan](assets/0123456789abcdef.png)\n");
    const handle = host.querySelector<HTMLButtonElement>("button[aria-label='Resize from the right edge']")!;
    // jsdom has no PointerEvent; MouseEvent with clientX does for our use.
    const pointer = (type: string, target: EventTarget, x: number) => target.dispatchEvent(new MouseEvent(type, { clientX: x, bubbles: true, cancelable: true }));
    await act(async () => { pointer("pointerdown", handle, 500); });
    await act(async () => { pointer("pointermove", window, 700); });
    await act(async () => { pointer("pointerup", window, 700); });
    await settle();
    await commit();
    // jsdom offers no layout, so the drag clamps to the smallest width (120px).
    expect(onChange.mock.calls[0][0]).toBe("Intro\n\n![Plan](assets/0123456789abcdef.png \"120\")\n");
  });
});

describe("rich fences", () => {
  const MATH = "Before\n\n```math\nx^2 + y^2\n```\n\nAfter\n";
  const figure = () => host.querySelector("[data-rich-fence]");
  const source = () => host.querySelector<HTMLPreElement>(".note-editor pre")!;
  // jsdom focuses only an element with a tabindex, never a bare
  // contenteditable, and runs no frame for tiptap's deferred focus; a browser
  // needs neither.
  const focusable = async () => {
    editor().view.dom.setAttribute("tabindex", "-1");
    await act(async () => { editor().view.dom.focus(); });
  };
  /** Put the cursor inside the first code block, with the editor focused. */
  const enterFence = async () => {
    let at = -1;
    editor().state.doc.descendants((node, pos) => { if (at < 0 && node.type.name === "codeBlock") at = pos; });
    await focusable();
    await act(async () => { editor().chain().focus().setTextSelection(at + 2).run(); });
    await settle();
  };

  it("round-trips a mermaid fence unchanged", async () => {
    const text = "Intro\n\n```mermaid\ngraph TD; A-->B\n```\n";
    const { onChange } = await mount(text);
    await typeAfter("Intro", "!");
    await commit();
    expect(onChange).toHaveBeenLastCalledWith(text.replace("Intro", "Intro!"));
  });

  it("shows the figure for a rich fence and hides its source", async () => {
    await mount(MATH);
    expect(figure()?.querySelector(".katex")).not.toBeNull();
    expect(source().hidden).toBe(true);
    expect(source().textContent).toBe("x^2 + y^2");
  });

  it("shows the source while the cursor is inside", async () => {
    await mount(MATH);
    await enterFence();
    expect(figure()).toBeNull();
    expect(source().hidden).toBe(false);
    await act(async () => { editor().chain().focus().setTextSelection(2).run(); });
    await settle();
    expect(figure()).not.toBeNull();
  });

  it("clicking the figure moves the cursor into the source", async () => {
    await mount(MATH);
    await focusable();
    await act(async () => { figure()!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); });
    await settle();
    expect(editor().isActive("codeBlock")).toBe(true);
    expect(figure()).toBeNull();
    expect(source().hidden).toBe(false);
  });

  it("leaves an ordinary code block as code", async () => {
    await mount("```ts\nconst x = 1;\n```\n");
    expect(figure()).toBeNull();
    expect(source().hidden).toBe(false);
    expect(source().textContent).toBe("const x = 1;");
  });

  describe("drawings", () => {
    const SCENE = '{"type":"excalidraw","version":2,"source":"bridge","elements":[\n{"id":"a","type":"rectangle","x":0,"y":0,"width":10,"height":10,"version":3}\n],"appState":{},"files":{}}';
    const PAGE = `Before\n\n\`\`\`excalidraw\n${SCENE}\n\`\`\`\n\nAfter\n`;
    const drawingBlock = () => host.querySelector("[data-drawing]");
    const pressDrawing = async () => {
      await act(async () => { host.querySelector("[data-drawing] button")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); });
      await settle();
    };

    it("draws an excalidraw fence in place, hides its JSON, and round-trips it", async () => {
      const { onChange } = await mount(PAGE);
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
      expect(drawingBlock()?.textContent).toContain("1 elements");
      expect(source().hidden).toBe(true);
      await typeAfter("Before", "!");
      await commit();
      expect(onChange).toHaveBeenLastCalledWith(PAGE.replace("Before", "Before!"));
    });

    it("opens the editor on a click and saves the scene into the fence", async () => {
      const { onChange } = await mount(PAGE);
      await pressDrawing();
      expect(host.ownerDocument.querySelector("[data-drawing-editor]")).not.toBeNull();
      await act(async () => { [...host.ownerDocument.querySelectorAll("button")].find(node => node.textContent === "Save drawing")!.click(); });
      await commit();
      const saved = onChange.mock.calls.at(-1)![0] as string;
      expect(saved.startsWith("Before\n\n```excalidraw\n{")).toBe(true);
      expect(saved).toContain('{"id":"n","type":"rectangle"');
      expect(saved).toContain('{"id":"a","type":"rectangle"');
      expect(saved.endsWith("```\n\nAfter\n")).toBe(true);
      expect(host.ownerDocument.querySelector("[data-drawing-editor]")).toBeNull();
    });

    it("shows a fence that is not a scene as code", async () => {
      await mount("```excalidraw\nnot json\n```\n");
      expect(drawingBlock()).toBeNull();
      expect(source().hidden).toBe(false);
    });

    it("the slash menu inserts an empty drawing", async () => {
      const { onChange } = await mount("first\n");
      await act(async () => { editor().chain().focus("end").insertContent({ type: "paragraph" }).run(); });
      await act(async () => { BLOCK_CHOICES.find(choice => choice.id === "drawing")!.run(editor(), {} as never); });
      await commit();
      expect(onChange).toHaveBeenLastCalledWith('first\n\n```excalidraw\n{"type":"excalidraw","version":2,"source":"bridge","elements":[],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}}\n```\n');
      expect(drawingBlock()?.textContent).toContain("Empty drawing. Click to draw.");
    });
  });

  it("the slash menu inserts a mermaid block", async () => {
    const { onChange } = await mount("first\n");
    await act(async () => { editor().chain().focus("end").insertContent({ type: "paragraph" }).run(); });
    await act(async () => { BLOCK_CHOICES.find(choice => choice.id === "mermaid")!.run(editor(), {} as never); });
    await act(async () => { editor().commands.insertContent("graph TD; A-->B"); });
    await commit();
    expect(onChange).toHaveBeenLastCalledWith("first\n\n```mermaid\ngraph TD; A-->B\n```\n");
  });
});

