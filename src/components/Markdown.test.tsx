import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Markdown, renderMathToHtml, splitBlocks } from "./Markdown";

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
