import { memo, useEffect, useMemo, useState } from "react";
import { Check, Copy, Maximize2, Minimize2 } from "lucide-react";
import katex from "katex";
import { COLORIZE_DEBOUNCE_MS, colorizeCode, escapeHtml, normalizeLang } from "./highlight";
import { DiagramFigure, isValidDiagramSpec, type DiagramSpec } from "./DiagramFigure";

type Block =
  | { kind: "code"; lang: string; body: string }
  | { kind: "diagram"; spec: string }
  | { kind: "math"; tex: string }
  | { kind: "html"; html: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "table"; header: string[]; rows: string[][] }
  | { kind: "para"; text: string };

/** Split a `| a | b |` row into trimmed cells, dropping the leading/trailing pipe. */
function splitTableRow(line: string): string[] {
  const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  return trimmed.split("|").map(cell => cell.trim());
}

// A GFM header-separator row: cells of only dashes, with optional `:` alignment markers.
const TABLE_SEPARATOR = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

function isTableRow(line: string): boolean {
  return line.includes("|");
}

/** Classify a fenced block by its info string into a rich-content block kind. */
function fencedBlock(lang: string, body: string): Block {
  const key = lang.trim().toLowerCase();
  if (key === "diagram") return { kind: "diagram", spec: body };
  if (key === "math" || key === "latex" || key === "tex") return { kind: "math", tex: body };
  if (key === "html") return { kind: "html", html: body };
  return { kind: "code", lang, body };
}

export function splitBlocks(source: string): Block[] {
  const blocks: Block[] = [];
  const lines = source.replaceAll("\r\n", "\n").split("\n");
  let index = 0;
  while (index < lines.length) {
    const line = lines[index];
    const trimmed = line.trim();
    if (!trimmed) { index += 1; continue; }
    const fence = trimmed.match(/^```(\S*)/);
    if (fence) {
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index].trim().startsWith("```")) { body.push(lines[index]); index += 1; }
      index += 1;
      blocks.push(fencedBlock(fence[1] ?? "", body.join("\n")));
      continue;
    }
    if (trimmed.startsWith("$$")) {
      const single = trimmed.match(/^\$\$(.+?)\$\$$/);
      if (single) { blocks.push({ kind: "math", tex: single[1].trim() }); index += 1; continue; }
      const body: string[] = [];
      const head = trimmed.slice(2);
      if (head) body.push(head);
      index += 1;
      while (index < lines.length && !lines[index].trim().endsWith("$$")) { body.push(lines[index]); index += 1; }
      if (index < lines.length) {
        const tail = lines[index].trim().slice(0, -2);
        if (tail) body.push(tail);
        index += 1;
      }
      blocks.push({ kind: "math", tex: body.join("\n").trim() });
      continue;
    }
    const heading = trimmed.match(/^(#{1,6})\s+(.*)$/);
    if (heading) { blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] }); index += 1; continue; }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) { blocks.push({ kind: "rule" }); index += 1; continue; }
    if (/^>\s?/.test(trimmed)) {
      const quote: string[] = [];
      while (index < lines.length && /^>\s?/.test(lines[index].trim())) { quote.push(lines[index].trim().replace(/^>\s?/, "")); index += 1; }
      blocks.push({ kind: "quote", text: quote.join("\n") });
      continue;
    }
    if (isTableRow(trimmed) && index + 1 < lines.length && TABLE_SEPARATOR.test(lines[index + 1].trim()) && isTableRow(lines[index + 1].trim())) {
      const header = splitTableRow(trimmed);
      index += 2;
      const rows: string[][] = [];
      while (index < lines.length && isTableRow(lines[index].trim())) { rows.push(splitTableRow(lines[index])); index += 1; }
      blocks.push({ kind: "table", header, rows });
      continue;
    }
    const bullet = /^[-*+]\s+/; const numbered = /^\d+[.)]\s+/;
    if (bullet.test(trimmed) || numbered.test(trimmed)) {
      const ordered = numbered.test(trimmed);
      const marker = ordered ? numbered : bullet;
      const items: string[] = [];
      while (index < lines.length) {
        const current = lines[index].trim();
        if (marker.test(current)) { items.push(current.replace(marker, "")); index += 1; continue; }
        if (current && !bullet.test(current) && !numbered.test(current) && items.length && lines[index].startsWith("  ")) {
          items[items.length - 1] += ` ${current}`; index += 1; continue;
        }
        break;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    const para: string[] = [trimmed];
    index += 1;
    while (index < lines.length) {
      const current = lines[index].trim();
      const startsTable = isTableRow(current) && index + 1 < lines.length && TABLE_SEPARATOR.test(lines[index + 1].trim()) && isTableRow(lines[index + 1].trim());
      if (!current || current.startsWith("```") || current.startsWith("$$") || /^(#{1,6})\s+/.test(current) || bullet.test(current) || numbered.test(current) || /^>\s?/.test(current) || startsTable) break;
      para.push(current); index += 1;
    }
    blocks.push({ kind: "para", text: para.join("\n") });
  }
  return blocks;
}

// Inline tokens, in priority order: code span, \(math\), $math$, bold, italic, link.
// The $…$ pattern requires non-space just inside both delimiters and forbids a
// trailing digit, so ordinary prose ("costs $5 and $10") is not misread as math.
const INLINE = /(`[^`]+`|\\\([^\n]*?\\\)|\$(?![\s$])(?:[^\n$]*?[^\s$])?\$(?!\d)|~~[^~\n]+~~|\*\*[^*]+\*\*|\*[^*\n]+\*|\[[^\]]+\]\([^)\s]+\))/g;

/** Render a LaTeX string to KaTeX HTML, or null if it cannot be parsed. */
export function renderMathToHtml(tex: string, displayMode: boolean): string | null {
  try {
    return katex.renderToString(tex, { displayMode, throwOnError: true, trust: false, output: "htmlAndMathml" });
  } catch {
    return null;
  }
}

function InlineMath({ tex }: { tex: string }) {
  const html = useMemo(() => renderMathToHtml(tex, false), [tex]);
  if (html == null) return <code>{tex}</code>;
  return <span dangerouslySetInnerHTML={{ __html: html }} />;
}

/**
 * The `dark` class on <html> is the single source of truth for the theme.
 * The sandboxed iframe renders outside our token scope, so it has to follow
 * it explicitly instead of inheriting CSS variables.
 */
function useDarkTheme(): boolean {
  const [dark, setDark] = useState(() => typeof document !== "undefined" && document.documentElement.classList.contains("dark"));
  useEffect(() => {
    const sync = () => setDark(document.documentElement.classList.contains("dark"));
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return dark;
}

/** Shared copy-to-clipboard state for the code/math/mermaid copy affordances. */
function useCopy(text: string) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1400); });
  };
  return { copied, copy };
}

function CopyButton({ text, className }: { text: string; className: string }) {
  const { copied, copy } = useCopy(text);
  return (
    <button type="button" className={className} onClick={copy} aria-label={copied ? "Copied" : "Copy"}>
      {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

function MathBlock({ tex }: { tex: string }) {
  const html = useMemo(() => renderMathToHtml(tex, true), [tex]);
  if (html == null) {
    return <pre className="my-[0.6em] overflow-x-auto rounded-[0.7rem] border border-destructive/30 bg-destructive/10 px-[0.85em] py-[0.6em] text-destructive"><code>{tex}</code></pre>;
  }
  return (
    <div className="rich-block my-[0.9em]">
      <CopyButton text={tex} className="rich-block-copy" />
      <div className="overflow-x-auto py-[0.2em] text-foreground" dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}

function renderInline(text: string): React.ReactNode[] {
  return text.split(INLINE).filter(part => part !== "").map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) return <code key={index}>{part.slice(1, -1)}</code>;
    if (part.startsWith("\\(") && part.endsWith("\\)") && part.length > 4) return <InlineMath key={index} tex={part.slice(2, -2)} />;
    if (part.startsWith("$") && part.endsWith("$") && part.length > 2) return <InlineMath key={index} tex={part.slice(1, -1)} />;
    if (part.startsWith("~~") && part.endsWith("~~") && part.length > 4) return <del key={index}>{renderInline(part.slice(2, -2))}</del>;
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) return <strong key={index}>{renderInline(part.slice(2, -2))}</strong>;
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2) return <em key={index}>{renderInline(part.slice(1, -1))}</em>;
    const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
    if (link) return <a key={index} href={link[2]} target="_blank" rel="noreferrer">{renderInline(link[1])}</a>;
    return <span key={index}>{part}</span>;
  });
}

function CodeBlock({ lang, body }: { lang: string; body: string }) {
  // `html` is derived at render time, not reset by an effect: an effect only
  // runs after commit, so for one real paint a naive `useEffect`-driven reset
  // would show the *previous* block's coloured HTML under the *new* body.
  // Comparing the cache against the current props keeps that impossible —
  // the very first render after a change already falls back to plain.
  const [cache, setCache] = useState<{ body: string; lang: string; html: string } | null>(null);
  const html = cache && cache.body === body && cache.lang === lang ? cache.html : escapeHtml(body);

  useEffect(() => {
    let live = true;
    // Debounced: see `COLORIZE_DEBOUNCE_MS` — a streaming reply re-renders
    // this on every delta, and a still-growing fence shouldn't schedule a
    // tokenization pass for every intermediate length.
    const timer = window.setTimeout(() => {
      void colorizeCode(body, lang).then(result => { if (live) setCache({ body, lang, html: result }); });
    }, COLORIZE_DEBOUNCE_MS);
    return () => { live = false; window.clearTimeout(timer); };
  }, [body, lang]);
  const label = normalizeLang(lang) || lang.toLowerCase() || "text";
  return (
    <div className="code-block">
      <div className="code-block-header">
        <span className="code-block-lang">{label}</span>
        <CopyButton text={body} className="code-block-copy" />
      </div>
      <pre><code className="stx" dangerouslySetInnerHTML={{ __html: html }} /></pre>
    </div>
  );
}

function DiagramBlock({ spec }: { spec: string }) {
  const parsed = useMemo<DiagramSpec | null>(() => {
    try {
      const value: unknown = JSON.parse(spec);
      return isValidDiagramSpec(value) ? value : null;
    } catch {
      return null;
    }
  }, [spec]);

  if (!parsed) {
    return (
      <div className="my-[0.8em]">
        <div className="mb-[0.35em] text-xs text-warning">Could not render this diagram — showing its source.</div>
        <CodeBlock lang="diagram" body={spec} />
      </div>
    );
  }
  return (
    <div className="rich-block my-[0.9em]">
      <CopyButton text={spec} className="rich-block-copy" />
      <DiagramFigure spec={parsed} />
    </div>
  );
}

// Agent-authored HTML is untrusted. Rendering happens inside a fully sandboxed
// iframe: sandbox="" grants no capabilities (no scripts, no same-origin), which
// is the sole isolation boundary because the Tauri webview sets no CSP.
// The sandboxed document has no stylesheet of its own, so its `color-scheme`
// is pinned to the active theme — that is what makes the UA's default text
// legible on the token background in both modes.
function HtmlBlock({ html }: { html: string }) {
  const dark = useDarkTheme();
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") setFullscreen(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen]);

  const frame = (
    <iframe
      className={`w-full flex-1 border-0 bg-background ${dark ? "[color-scheme:dark]" : "[color-scheme:light]"}`}
      title="Rendered HTML"
      sandbox=""
      referrerPolicy="no-referrer"
      srcDoc={html}
    />
  );

  if (fullscreen) {
    return (
      <div className="fixed inset-0 z-50 flex flex-col bg-scrim p-4 backdrop-blur-md sm:p-8">
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[0.9rem] border border-border bg-background">
          <div className="code-block-header">
            <span className="code-block-lang">html</span>
            <button type="button" className="code-block-copy" onClick={() => setFullscreen(false)} aria-label="Exit fullscreen" title="Exit fullscreen (Esc)">
              <Minimize2 size={12} aria-hidden="true" />
              Close
            </button>
          </div>
          {frame}
        </div>
      </div>
    );
  }

  return (
    <div className="html-block my-[0.9em]">
      <div className="code-block-header">
        <span className="code-block-lang">html</span>
        <button type="button" className="code-block-copy" onClick={() => setFullscreen(true)} aria-label="Fullscreen" title="Fullscreen">
          <Maximize2 size={12} aria-hidden="true" />
          Expand
        </button>
      </div>
      {frame}
    </div>
  );
}

export const Markdown = memo(function Markdown({ text, dim }: { text: string; dim?: boolean }) {
  return (
    <div className={dim ? "md dim" : "md"}>
      {splitBlocks(text).map((block, index) => {
        if (block.kind === "code") return <CodeBlock key={index} lang={block.lang} body={block.body} />;
        if (block.kind === "diagram") return <DiagramBlock key={index} spec={block.spec} />;
        if (block.kind === "math") return <MathBlock key={index} tex={block.tex} />;
        if (block.kind === "html") return <HtmlBlock key={index} html={block.html} />;
        if (block.kind === "heading") {
          const H = (`h${Math.min(block.level, 4)}`) as keyof JSX.IntrinsicElements;
          return <H key={index}>{renderInline(block.text)}</H>;
        }
        if (block.kind === "rule") return <hr key={index} />;
        if (block.kind === "quote") return <blockquote key={index}>{renderInline(block.text)}</blockquote>;
        if (block.kind === "table") {
          return (
            <table key={index}>
              <thead><tr>{block.header.map((cell, cellIndex) => <th key={cellIndex}>{renderInline(cell)}</th>)}</tr></thead>
              <tbody>
                {block.rows.map((row, rowIndex) => (
                  <tr key={rowIndex}>{row.map((cell, cellIndex) => <td key={cellIndex}>{renderInline(cell)}</td>)}</tr>
                ))}
              </tbody>
            </table>
          );
        }
        if (block.kind === "list") {
          const List = block.ordered ? "ol" : "ul";
          return <List key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}>{renderInline(item)}</li>)}</List>;
        }
        return <p key={index}>{renderInline(block.text)}</p>;
      })}
    </div>
  );
});
