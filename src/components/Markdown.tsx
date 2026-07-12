import { useState } from "react";
import { Check, Copy } from "lucide-react";

// Minimal, dependency-free markdown renderer for agent prose: paragraphs,
// headings, lists, blockquotes, fenced code blocks and inline code/bold/
// italic/links. Builds React nodes directly — no HTML injection.

type Block =
  | { kind: "code"; lang: string; body: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "para"; text: string };

function splitBlocks(source: string): Block[] {
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
      blocks.push({ kind: "code", lang: fence[1] ?? "", body: body.join("\n") });
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
        // continuation line of the previous item
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
      if (!current || current.startsWith("```") || /^(#{1,6})\s+/.test(current) || bullet.test(current) || numbered.test(current) || /^>\s?/.test(current)) break;
      para.push(current); index += 1;
    }
    blocks.push({ kind: "para", text: para.join("\n") });
  }
  return blocks;
}

// Inline tokens: `code`, **bold**, *italic*, [label](url)
const INLINE = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\n]+\*|\[[^\]]+\]\([^)\s]+\))/g;

function renderInline(text: string): React.ReactNode[] {
  return text.split(INLINE).filter(part => part !== "").map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) return <code className="md-code" key={index}>{part.slice(1, -1)}</code>;
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) return <strong key={index}>{renderInline(part.slice(2, -2))}</strong>;
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2) return <em key={index}>{renderInline(part.slice(1, -1))}</em>;
    const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
    if (link) return <a className="md-link" key={index} href={link[2]} target="_blank" rel="noreferrer">{renderInline(link[1])}</a>;
    return <span key={index}>{part}</span>;
  });
}

function CodeBlock({ lang, body }: { lang: string; body: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(body).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1400); });
  };
  return <div className="md-block">
    <div className="md-block-bar"><span>{lang || "text"}</span><button onClick={copy} title="Copy">{copied ? <Check size={12}/> : <Copy size={12}/>}</button></div>
    <pre><code>{body}</code></pre>
  </div>;
}

export function Markdown({ text }: { text: string }) {
  return <>{splitBlocks(text).map((block, index) => {
    if (block.kind === "code") return <CodeBlock key={index} lang={block.lang} body={block.body}/>;
    if (block.kind === "heading") { const H = (`h${Math.min(block.level + 2, 6)}`) as keyof JSX.IntrinsicElements; return <H className="md-heading" key={index}>{renderInline(block.text)}</H>; }
    if (block.kind === "rule") return <hr className="md-rule" key={index}/>;
    if (block.kind === "quote") return <blockquote className="md-quote" key={index}>{renderInline(block.text)}</blockquote>;
    if (block.kind === "list") {
      const List = block.ordered ? "ol" : "ul";
      return <List className="md-list" key={index}>{block.items.map((item, itemIndex) => <li key={itemIndex}>{renderInline(item)}</li>)}</List>;
    }
    return <p className="md-para" key={index}>{renderInline(block.text)}</p>;
  })}</>;
}
