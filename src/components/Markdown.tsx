import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";
import katex from "katex";
import { highlightCode, normalizeLang } from "./highlight";

type Block =
  | { kind: "code"; lang: string; body: string }
  | { kind: "mermaid"; code: string }
  | { kind: "math"; tex: string }
  | { kind: "html"; html: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "para"; text: string };

/** Classify a fenced block by its info string into a rich-content block kind. */
function fencedBlock(lang: string, body: string): Block {
  const key = lang.trim().toLowerCase();
  if (key === "mermaid") return { kind: "mermaid", code: body };
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
      if (!current || current.startsWith("```") || current.startsWith("$$") || /^(#{1,6})\s+/.test(current) || bullet.test(current) || numbered.test(current) || /^>\s?/.test(current)) break;
      para.push(current); index += 1;
    }
    blocks.push({ kind: "para", text: para.join("\n") });
  }
  return blocks;
}

// Inline tokens, in priority order: code span, \(math\), $math$, bold, italic, link.
// The $…$ pattern requires non-space just inside both delimiters and forbids a
// trailing digit, so ordinary prose ("costs $5 and $10") is not misread as math.
const INLINE = /(`[^`]+`|\\\([^\n]*?\\\)|\$(?![\s$])(?:[^\n$]*?[^\s$])?\$(?!\d)|\*\*[^*]+\*\*|\*[^*\n]+\*|\[[^\]]+\]\([^)\s]+\))/g;

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

function MathBlock({ tex }: { tex: string }) {
  const html = useMemo(() => renderMathToHtml(tex, true), [tex]);
  if (html == null) {
    return <pre className="my-[0.6em] overflow-x-auto rounded-[0.7rem] border border-red-400/35 bg-red-950/20 px-[0.85em] py-[0.6em] text-red-300"><code>{tex}</code></pre>;
  }
  return <div className="my-[0.9em] overflow-x-auto py-[0.2em] text-foreground" dangerouslySetInnerHTML={{ __html: html }} />;
}

function renderInline(text: string): React.ReactNode[] {
  return text.split(INLINE).filter(part => part !== "").map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) return <code key={index}>{part.slice(1, -1)}</code>;
    if (part.startsWith("\\(") && part.endsWith("\\)") && part.length > 4) return <InlineMath key={index} tex={part.slice(2, -2)} />;
    if (part.startsWith("$") && part.endsWith("$") && part.length > 2) return <InlineMath key={index} tex={part.slice(1, -1)} />;
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) return <strong key={index}>{renderInline(part.slice(2, -2))}</strong>;
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2) return <em key={index}>{renderInline(part.slice(1, -1))}</em>;
    const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
    if (link) return <a key={index} href={link[2]} target="_blank" rel="noreferrer">{renderInline(link[1])}</a>;
    return <span key={index}>{part}</span>;
  });
}

function CodeBlock({ lang, body }: { lang: string; body: string }) {
  const [copied, setCopied] = useState(false);
  const highlighted = useMemo(() => highlightCode(body, lang), [body, lang]);
  const label = normalizeLang(lang) || lang.toLowerCase() || "text";
  const copy = () => {
    void navigator.clipboard?.writeText(body).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1400); });
  };
  return (
    <div className="code-block">
      <div className="code-block-header">
        <span className="code-block-lang">{label}</span>
        <button type="button" className="code-block-copy" onClick={copy}>
          {copied ? <Check size={12} aria-hidden="true" /> : <Copy size={12} aria-hidden="true" />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre><code className="hljs" dangerouslySetInnerHTML={{ __html: highlighted }} /></pre>
    </div>
  );
}

let mermaidSeq = 0;

function MermaidBlock({ code }: { code: string }) {
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const idRef = useRef("");
  if (!idRef.current) { mermaidSeq += 1; idRef.current = `bridge-mermaid-${mermaidSeq}`; }

  useEffect(() => {
    let cancelled = false;
    setSvg(null);
    setFailed(false);
    (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        mermaid.initialize({ startOnLoad: false, theme: "dark", securityLevel: "strict" });
        await mermaid.parse(code); // throws on malformed diagrams
        const rendered = await mermaid.render(idRef.current, code);
        if (!cancelled) setSvg(rendered.svg);
      } catch {
        if (!cancelled) setFailed(true);
      }
    })();
    return () => { cancelled = true; };
  }, [code]);

  if (failed) {
    return (
      <div className="my-[0.8em]">
        <div className="mb-[0.35em] text-xs text-amber-300">Could not render this Mermaid diagram — showing its source.</div>
        <CodeBlock lang="mermaid" body={code} />
      </div>
    );
  }
  if (svg == null) {
    return <div className="my-[0.9em] rounded-[0.9rem] border border-dashed border-border p-[0.9em_1em] text-xs text-muted-foreground">Rendering diagram…</div>;
  }
  return <div className="my-[0.9em] flex justify-center overflow-x-auto [&_svg]:h-auto [&_svg]:max-w-full" role="img" dangerouslySetInnerHTML={{ __html: svg }} />;
}

// Agent-authored HTML is untrusted. Rendering happens inside a fully sandboxed
// iframe: sandbox="" grants no capabilities (no scripts, no same-origin), which
// is the sole isolation boundary because the Tauri webview sets no CSP.
function HtmlBlock({ html }: { html: string }) {
  return (
    <iframe
      className="my-[0.9em] min-h-30 w-full rounded-[0.9rem] border border-border bg-white [color-scheme:light]"
      title="Rendered HTML"
      sandbox=""
      referrerPolicy="no-referrer"
      srcDoc={html}
    />
  );
}

export const Markdown = memo(function Markdown({ text, dim }: { text: string; dim?: boolean }) {
  return (
    <div className={dim ? "md dim" : "md"}>
      {splitBlocks(text).map((block, index) => {
        if (block.kind === "code") return <CodeBlock key={index} lang={block.lang} body={block.body} />;
        if (block.kind === "mermaid") return <MermaidBlock key={index} code={block.code} />;
        if (block.kind === "math") return <MathBlock key={index} tex={block.tex} />;
        if (block.kind === "html") return <HtmlBlock key={index} html={block.html} />;
        if (block.kind === "heading") {
          const H = (`h${Math.min(block.level, 4)}`) as keyof JSX.IntrinsicElements;
          return <H key={index}>{renderInline(block.text)}</H>;
        }
        if (block.kind === "rule") return <hr key={index} />;
        if (block.kind === "quote") return <blockquote key={index}>{renderInline(block.text)}</blockquote>;
        if (block.kind === "list") {
          const List = block.ordered ? "ol" : "ul";
          return <List key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}>{renderInline(item)}</li>)}</List>;
        }
        return <p key={index}>{renderInline(block.text)}</p>;
      })}
    </div>
  );
});
