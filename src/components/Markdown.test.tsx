// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Markdown, renderMathToHtml, splitBlocks } from "./Markdown";

vi.mock("mermaid", () => ({
  default: {
    initialize: vi.fn(),
    parse: vi.fn().mockResolvedValue(true),
    render: vi.fn().mockResolvedValue({ svg: '<svg data-testid="diagram"></svg>' }),
  },
}));

describe("splitBlocks rich content detection", () => {
  it("detects a mermaid fenced block", () => {
    const blocks = splitBlocks("```mermaid\ngraph TD; A-->B;\n```");
    expect(blocks).toEqual([{ kind: "mermaid", code: "graph TD; A-->B;" }]);
  });

  it("detects an html fenced block", () => {
    const blocks = splitBlocks("```html\n<b>hi</b>\n```");
    expect(blocks).toEqual([{ kind: "html", html: "<b>hi</b>" }]);
  });

  it("detects a math fenced block and its aliases", () => {
    expect(splitBlocks("```math\nx^2\n```")).toEqual([{ kind: "math", tex: "x^2" }]);
    expect(splitBlocks("```latex\nx^2\n```")).toEqual([{ kind: "math", tex: "x^2" }]);
  });

  it("detects $$ display math, single-line and multi-line", () => {
    expect(splitBlocks("$$a=b$$")).toEqual([{ kind: "math", tex: "a=b" }]);
    expect(splitBlocks("$$\n\\int_0^1 x\\,dx\n$$")).toEqual([{ kind: "math", tex: "\\int_0^1 x\\,dx" }]);
  });

  it("leaves ordinary fenced code untouched", () => {
    expect(splitBlocks("```ts\nconst x = 1;\n```")).toEqual([{ kind: "code", lang: "ts", body: "const x = 1;" }]);
  });

  it("does not misread prices as inline math", () => {
    const html = renderToStaticMarkup(<Markdown text="It costs $5 and $10 today." />);
    expect(html).toContain("It costs $5 and $10 today.");
    expect(html).not.toContain("katex");
  });

  it("detects a GFM pipe table with a header-separator row", () => {
    const blocks = splitBlocks("| Name | Age |\n| --- | --- |\n| Ann | 30 |\n| Bo | 41 |");
    expect(blocks).toEqual([{
      kind: "table",
      header: ["Name", "Age"],
      rows: [["Ann", "30"], ["Bo", "41"]],
    }]);
  });

  it("does not treat pipe-containing lines as a table without a header-separator row", () => {
    const blocks = splitBlocks("a | b\nc | d");
    expect(blocks.every(block => block.kind !== "table")).toBe(true);
  });

  it("still treats a lone dash line as a rule, not a table separator", () => {
    expect(splitBlocks("---")).toEqual([{ kind: "rule" }]);
  });
});

describe("GFM table rendering", () => {
  it("renders a pipe table as a real <table> with <thead>/<tbody>, not a paragraph of pipes", () => {
    const html = renderToStaticMarkup(<Markdown text={"| Name | Age |\n| --- | --- |\n| Ann | 30 |"} />);
    expect(html).toContain("<table");
    expect(html).toContain("<thead");
    expect(html).toContain("<tbody");
    expect(html).toMatch(/<th[^>]*>.*Name.*<\/th>/);
    expect(html).toMatch(/<td[^>]*>.*Ann.*<\/td>/);
    expect(html).not.toMatch(/<p>[^<]*\|/);
  });

  it("renders ~~text~~ as <del> strikethrough inline", () => {
    const html = renderToStaticMarkup(<Markdown text="this is ~~wrong~~ but this is right" />);
    expect(html).toMatch(/<del>.*wrong.*<\/del>/);
  });
});

describe("renderMathToHtml fallback", () => {
  it("renders valid LaTeX to KaTeX html", () => {
    const html = renderMathToHtml("x^2 + y^2", true);
    expect(html).not.toBeNull();
    expect(html).toContain("katex");
  });

  it("returns null on invalid LaTeX so callers can fall back to raw text", () => {
    expect(renderMathToHtml("\\frac{1}{", false)).toBeNull();
  });
});

describe("HtmlBlock sandbox isolation", () => {
  it("renders untrusted html inside a fully sandboxed iframe", () => {
    const html = renderToStaticMarkup(<Markdown text={"```html\n<h1>Report</h1>\n```"} />);
    expect(html).toContain('sandbox=""');
    // The empty sandbox must never be widened; scripts and same-origin stay off.
    expect(html).not.toContain("allow-scripts");
    expect(html).not.toContain("allow-same-origin");
  });

  it("gives up a sensible default framing and a fullscreen toggle instead of the tiny min-h-30", () => {
    const html = renderToStaticMarkup(<Markdown text={"```html\n<h1>Report</h1>\n```"} />);
    expect(html).not.toContain("min-h-30");
    expect(html).toContain("html-block");
    expect(html).toContain('aria-label="Fullscreen"');
  });
});

describe("copy affordances (interactive)", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      configurable: true,
    });
  });

  afterEach(() => {
    act(() => { root.unmount(); });
    container.remove();
  });

  it("CodeBlock's copy button still copies the raw code", async () => {
    await act(async () => { root.render(<Markdown text={"```ts\nconst x = 1;\n```"} />); });
    const button = container.querySelector(".code-block-copy") as HTMLButtonElement;
    expect(button).toBeTruthy();
    await act(async () => { button.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("const x = 1;");
    expect(button.textContent).toContain("Copied");
  });

  it("MathBlock's copy button copies the raw LaTeX source", async () => {
    await act(async () => { root.render(<Markdown text={"$$x^2$$"} />); });
    const button = container.querySelector(".rich-block-copy") as HTMLButtonElement;
    expect(button).toBeTruthy();
    await act(async () => { button.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("x^2");
    expect(button.textContent).toContain("Copied");
  });

  it("MermaidBlock's copy button copies the raw diagram source", async () => {
    const code = "graph TD; A-->B;";
    await act(async () => { root.render(<Markdown text={`\`\`\`mermaid\n${code}\n\`\`\``} />); });
    // Flush the dynamic import + async parse/render chain before the copy button exists.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
    const button = container.querySelector(".rich-block-copy") as HTMLButtonElement;
    expect(button).toBeTruthy();
    await act(async () => { button.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(code);
    expect(button.textContent).toContain("Copied");
  });

  it("HtmlBlock's fullscreen toggle opens and closes an overlay without weakening the sandbox", async () => {
    await act(async () => { root.render(<Markdown text={"```html\n<h1>hi</h1>\n```"} />); });
    const open = container.querySelector('[aria-label="Fullscreen"]') as HTMLButtonElement;
    expect(open).toBeTruthy();

    await act(async () => { open.click(); });
    const iframe = container.querySelector("iframe") as HTMLIFrameElement;
    expect(iframe.getAttribute("sandbox")).toBe("");
    expect(iframe.getAttribute("allow")).toBeNull();

    const close = container.querySelector('[aria-label="Exit fullscreen"]') as HTMLButtonElement;
    expect(close).toBeTruthy();
    await act(async () => { close.click(); });
    expect(container.querySelector('[aria-label="Exit fullscreen"]')).toBeNull();
    expect(container.querySelector('[aria-label="Fullscreen"]')).toBeTruthy();
  });
});

describe("existing markdown behavior is preserved", () => {
  it("still renders headings, lists, and inline styles", () => {
    const html = renderToStaticMarkup(<Markdown text={"# Title\n\n- one\n- two\n\n**bold** and `code`"} />);
    expect(html).toContain("<h1>");
    expect(html).toContain("<ul>");
    expect(html).toContain("<strong>");
    expect(html).toContain("<code>code</code>");
  });
});
