// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FileLinkContext, Markdown, renderMathToHtml, splitBlocks, type FileLinks } from "./Markdown";

describe("splitBlocks rich content detection", () => {
  it("detects a diagram fenced block", () => {
    const spec = '{"nodes":[],"edges":[],"caption":"c","ariaLabel":"a"}';
    const blocks = splitBlocks("```diagram\n" + spec + "\n```");
    expect(blocks).toEqual([{ kind: "diagram", spec }]);
  });

  it("classifies a legacy mermaid fence as plain code, not a diagram", () => {
    expect(splitBlocks("```mermaid\ngraph TD; A-->B;\n```")).toEqual([
      { kind: "code", lang: "mermaid", body: "graph TD; A-->B;" },
    ]);
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

describe("DiagramBlock rendering", () => {
  it("renders a valid diagram spec as a labeled, captioned SVG", () => {
    const spec = JSON.stringify({
      nodes: [
        { id: "a", row: 0, col: 0, label: "start" },
        { id: "b", row: 1, col: 0, emphasis: "active", marker: "tip" },
      ],
      edges: [{ from: "a", to: "b", emphasis: "active" }],
      caption: "A grows into B.",
      ariaLabel: "Diagram: A grows into B.",
    });
    const html = renderToStaticMarkup(<Markdown text={"```diagram\n" + spec + "\n```"} />);
    expect(html).toContain('role="img"');
    expect(html).toContain("Diagram: A grows into B.");
    expect(html).toContain("A grows into B.");
    expect(html).toContain("start");
  });

  it("falls back to a labeled code block when the diagram JSON is malformed", () => {
    const html = renderToStaticMarkup(<Markdown text={"```diagram\nnot json\n```"} />);
    expect(html).toContain("Could not render this diagram");
    expect(html).toContain("code-block");
  });

  it("falls back to source when an edge references a node id that doesn't exist", () => {
    const spec = JSON.stringify({
      nodes: [{ id: "a", row: 0, col: 0 }],
      edges: [{ from: "a", to: "ghost" }],
      caption: "c",
      ariaLabel: "a",
    });
    const html = renderToStaticMarkup(<Markdown text={"```diagram\n" + spec + "\n```"} />);
    expect(html).toContain("Could not render this diagram");
  });

  it("no longer treats a legacy mermaid block as a failed diagram — just a plain code block", () => {
    const html = renderToStaticMarkup(<Markdown text={"```mermaid\ngraph TD; A-->B;\n```"} />);
    expect(html).toContain("code-block");
    expect(html).toContain("mermaid");
    expect(html).not.toContain("Could not render");
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

  it("DiagramBlock's copy button copies the raw JSON source", async () => {
    const spec = JSON.stringify({
      nodes: [{ id: "a", row: 0, col: 0 }],
      edges: [],
      caption: "c",
      ariaLabel: "a",
    });
    await act(async () => { root.render(<Markdown text={`\`\`\`diagram\n${spec}\n\`\`\``} />); });
    const button = container.querySelector(".rich-block-copy") as HTMLButtonElement;
    expect(button).toBeTruthy();
    await act(async () => { button.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(spec);
    expect(button.textContent).toContain("Copied");
  });

  it("HtmlBlock's fullscreen toggle opens and closes an overlay without weakening the sandbox", async () => {
    await act(async () => { root.render(<Markdown text={"```html\n<h1>hi</h1>\n```"} />); });
    const open = container.querySelector('[aria-label="Fullscreen"]') as HTMLButtonElement;
    expect(open).toBeTruthy();

    await act(async () => { open.click(); });
    // The overlay is portalled to document.body so no transformed ancestor can
    // become its `position: fixed` containing block and clip it to the bubble.
    // It therefore lives outside the message container, not inside it.
    expect(container.querySelector('[aria-label="Exit fullscreen"]')).toBeNull();
    const overlay = document.body.querySelector('[role="dialog"][aria-label="HTML preview"]') as HTMLElement;
    expect(overlay).toBeTruthy();
    expect(container.contains(overlay)).toBe(false);
    const iframe = overlay.querySelector("iframe") as HTMLIFrameElement;
    expect(iframe.getAttribute("sandbox")).toBe("");
    expect(iframe.getAttribute("allow")).toBeNull();

    const close = document.body.querySelector('[aria-label="Exit fullscreen"]') as HTMLButtonElement;
    expect(close).toBeTruthy();
    await act(async () => { close.click(); });
    expect(document.body.querySelector('[aria-label="Exit fullscreen"]')).toBeNull();
    expect(container.querySelector('[aria-label="Fullscreen"]')).toBeTruthy();
  });
});

describe("CodeBlock async colorization", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => { root.unmount(); });
    container.remove();
  });

  // Real dynamic-import + real Shiki tokenization, timed against the actual
  // wall clock — under a full, concurrent test-suite run, a single
  // `setTimeout(0)` tick isn't a reliable wait. Poll instead of guessing a
  // fixed delay.
  const waitFor = async (check: () => boolean, timeoutMs = 3000) => {
    const start = Date.now();
    while (!check()) {
      if (Date.now() - start > timeoutMs) throw new Error("timed out waiting for colorization");
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
    }
  };

  it("renders plain escaped text immediately, then upgrades to .stx-* spans", async () => {
    // A plain (non-async) act() flushes the effect's synchronous first half
    // (the immediate `setHtml(escapeHtml(body))`) without waiting for the
    // `colorizeCode` promise it also kicks off — the only way to observe the
    // pre-colour frame deterministically, independent of how warm the
    // shared Shiki module cache happens to be from earlier tests.
    act(() => { root.render(<Markdown text={"```ts\nconst x = 1;\n```"} />); });
    const code = container.querySelector("code.stx") as HTMLElement;
    expect(code).toBeTruthy();
    expect(code.innerHTML).toBe("const x = 1;");

    await waitFor(() => code.innerHTML.includes("stx-keyword"));
  });

  it("resets to plain text immediately when the code changes, instead of keeping stale colour", async () => {
    await act(async () => { root.render(<Markdown text={"```ts\nconst x = 1;\n```"} />); });
    const code = container.querySelector("code.stx") as HTMLElement;
    await waitFor(() => code.innerHTML.includes("stx-keyword"));
    expect(code.innerHTML).toContain("stx-keyword");

    act(() => { root.render(<Markdown text={"```ts\nconst y = 2;\n```"} />); });
    expect(code.innerHTML).toBe("const y = 2;");
  });
});

describe("link scheme allowlist", () => {
  it("only anchors schemes the shell will open, and keeps the rest as text", () => {
    // The Tauri webview sets no CSP and the click interceptor in
    // `externalLinks` only claims http(s)/mailto, so any other scheme would
    // keep the webview's default action — in-app execution for `javascript:`.
    for (const hostile of ["javascript:void%200", "data:text/html,<script>x</script>", "file:///etc/passwd"]) {
      const html = renderToStaticMarkup(<Markdown text={`a [click me](${hostile}) b`} />);
      expect(html).not.toContain("<a ");
      expect(html).not.toContain(hostile.split(":")[0] + ":");
      // The label survives, so the reader still sees what was written.
      expect(html).toContain("click me");
    }
    const safe = renderToStaticMarkup(<Markdown text={"[docs](https://example.test/x) and [mail](mailto:me@example.test)"} />);
    expect(safe).toContain('href="https://example.test/x"');
    expect(safe).toContain('href="mailto:me@example.test"');
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

describe("file links out of prose", () => {
  let container: HTMLDivElement;
  let root: Root;
  const open = vi.fn();
  const links: FileLinks = { has: path => ["src/App.tsx", "src/api.ts"].includes(path), open };

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    open.mockClear();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const mount = (text: string, value: FileLinks | null = links) => {
    act(() => {
      root.render(<FileLinkContext.Provider value={value}><Markdown text={text} /></FileLinkContext.Provider>);
    });
  };
  const click = (element: Element) => {
    act(() => {
      element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  };

  it("opens a workspace file named in inline code, at its line", () => {
    mount("The guard moved in `src/App.tsx:42` yesterday.");
    const button = container.querySelector('button[aria-label="Open src/App.tsx in the Code pane"]')!;
    expect(button.textContent).toBe("src/App.tsx:42");
    click(button);
    expect(open).toHaveBeenCalledWith("src/App.tsx", 42);
  });

  it("opens without a line when the code span has no suffix", () => {
    mount("See `src/api.ts` for the boundary.");
    click(container.querySelector('button[aria-label="Open src/api.ts in the Code pane"]')!);
    expect(open).toHaveBeenCalledWith("src/api.ts", undefined);
  });

  it("leaves an unresolvable path as plain code", () => {
    mount("See `src/nope.ts` for nothing.");
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector("code")!.textContent).toBe("src/nope.ts");
  });

  it("opens a mention in plain message text", () => {
    mount("please fix @src/App.tsx first");
    const button = container.querySelector('button[aria-label="Open src/App.tsx in the Code pane"]')!;
    expect(button.textContent).toBe("@src/App.tsx");
    click(button);
    expect(open).toHaveBeenCalledWith("src/App.tsx");
  });

  it("renders everything inert without a provider", () => {
    mount("see `src/App.tsx:42` and @src/App.tsx", null);
    expect(container.querySelector("button")).toBeNull();
    expect(container.textContent).toContain("src/App.tsx:42");
    expect(container.textContent).toContain("@src/App.tsx");
  });
});
