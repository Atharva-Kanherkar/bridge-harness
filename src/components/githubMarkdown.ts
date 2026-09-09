import { isExternalUrl } from "../externalLinks";

// GitHub-authored markdown is not the markdown Bridge's renderer was written
// for. A PR body arrives with `<details>` sections, HTML comments left behind
// by a template, `<img>` screenshots, `- [x]` task lists, bare URLs, and
// `#123`/`@user` cross-references — and the renderer printed every one of them
// literally, which is what "markdown doesn't render" actually looked like.
//
// This module is the translation layer, and it is deliberately *lossy in the
// safe direction*. Remote markup is never handed to the DOM: an allowlisted
// formatting tag is rewritten into its markdown equivalent, an image becomes a
// labelled link rather than a network fetch, a URL only becomes a link when
// its scheme is one `externalLinks` will hand to the OS browser, and everything
// else — `<script>` above all — is left exactly as authored so it renders as
// inert text through React the way it always has.

/** One run of a GitHub body: ordinary markdown, or a collapsible section. */
export type GithubBlock =
  | { kind: "markdown"; text: string }
  | { kind: "details"; summary: string; body: string };

/** Formatting tags GitHub allows in a comment that carry a markdown spelling.
 * Anything absent from this table stays verbatim text. */
const TAG_EQUIVALENT: Record<string, string> = {
  b: "**", strong: "**", i: "*", em: "*", code: "`", del: "~~", s: "~~", strike: "~~",
};

const FENCE_LINE = /^ {0,3}(?:```|~~~)/;

/**
 * Character ranges covered by fenced code blocks, fence lines included.
 *
 * Everything that rewrites a body consults this: a fenced block is *content*,
 * not markup, so a `<details>` example or a `#1` inside one has to survive
 * exactly as the author pasted it. An unclosed fence runs to the end of the
 * body, which is how GitHub renders it too.
 */
function fencedRanges(text: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  let offset = 0;
  let openedAt: number | null = null;
  for (const line of text.split("\n")) {
    if (FENCE_LINE.test(line.trim())) {
      if (openedAt === null) openedAt = offset;
      else { ranges.push([openedAt, offset + line.length]); openedAt = null; }
    }
    offset += line.length + 1;
  }
  if (openedAt !== null) ranges.push([openedAt, text.length]);
  return ranges;
}

function insideFence(index: number, ranges: Array<[number, number]>): boolean {
  return ranges.some(([start, end]) => index >= start && index < end);
}

// Inline code is masked out before any rewrite runs. Without it, a body that
// *documents* markup — ``show `<b>x</b>` and `<br>` `` — would have the
// contents of its own code spans rewritten, which is the opposite of what a
// code span means. The sentinels are private-use code points, and any that
// arrive in the source are stripped first so a body cannot forge a
// placeholder and smuggle text past the rewrites.
const MASK_OPEN = "\uE000";
const MASK_CLOSE = "\uE001";
const MASK_REF = /\uE000(\d+)\uE001/g;

function maskInlineCode(text: string): { masked: string; spans: string[] } {
  const spans: string[] = [];
  const masked = text.replace(/`[^`\n]*`/g, span => {
    spans.push(span);
    return `${MASK_OPEN}${spans.length - 1}${MASK_CLOSE}`;
  });
  return { masked, spans };
}

function unmaskInlineCode(text: string, spans: string[]): string {
  return text.replace(MASK_REF, (all, index: string) => spans[Number(index)] ?? all);
}

/**
 * The URL to link to, or `undefined` when nothing should be linked.
 *
 * A comment body is attacker-influenced and the pane renders it inside a Tauri
 * webview with no CSP, where the click interceptor in `externalLinks` only
 * claims `http(s)`/`mailto`. Any other scheme — `javascript:`, `data:`,
 * `file:` — would keep the webview's default action, so a rewritten `<a href>`
 * or `<img src>` is only ever allowed to become a link when that same
 * allowlist accepts it. Everything else keeps its label as inert text.
 */
function linkable(url: string): string | undefined {
  const trimmed = url.trim();
  return isExternalUrl(trimmed) ? trimmed : undefined;
}

/** A URL that is already the target of a markdown link or an autolink. */
const LINKED_URL = /(\]\([^)\s]*$|<[^>\s]*$)/;

const BARE_URL = /https?:\/\/[^\s<>()[\]"'`]+[^\s<>()[\]"'`.,;:!?]/g;

function autolink(part: string): string {
  let out = "";
  let last = 0;
  for (const match of part.matchAll(BARE_URL)) {
    const start = match.index ?? 0;
    out += part.slice(last, start);
    // Skip a URL that is already inside `](…)` or `<…>` — relinking it would
    // produce `[[url](url)](url)`.
    out += LINKED_URL.test(out) ? match[0] : `[${match[0]}](${match[0]})`;
    last = start + match[0].length;
  }
  return out + part.slice(last);
}

/** The origin a `@handle` belongs to. A GitHub Enterprise repository lives on
 * its own host, so deriving this from the repository URL is what keeps a
 * mention from linking to an unrelated github.com account. */
function accountOrigin(repositoryUrl: string): string | undefined {
  try {
    const origin = new URL(repositoryUrl).origin;
    return isExternalUrl(origin) ? origin : undefined;
  } catch {
    return undefined;
  }
}

function rewriteProse(source: string, repositoryUrl?: string): string {
  const { masked, spans } = maskInlineCode(source.replaceAll(MASK_OPEN, "").replaceAll(MASK_CLOSE, ""));
  let body = masked;

  // Template leftovers. An HTML comment is invisible on GitHub, so showing
  // it is strictly wrong rather than merely unstyled. A comment that owns its
  // whole line takes the line with it, or every stripped instruction block
  // would leave a gap where prose used to join up.
  body = body.replace(/^[ \t]*<!--[\s\S]*?-->[ \t]*\n?/gm, "");
  body = body.replace(/<!--[\s\S]*?-->/g, "");

  // `<br>` is the one tag whose meaning is purely a line break.
  body = body.replace(/<br\s*\/?>/gi, "\n");

  // A screenshot becomes a labelled link. Bridge does not fetch remote images
  // into the pane: a comment body is attacker-influenced, and an `<img src>`
  // is a network request (and a read receipt) we never promised.
  body = body.replace(/!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/g, (_all, alt: string, url: string) => {
    const label = alt.trim() ? `image: ${alt.trim()}` : "image";
    const target = linkable(url);
    return target ? `[${label}](${target})` : label;
  });
  body = body.replace(/<img\b[^>]*?src=["']([^"']+)["'][^>]*>/gi, (_all, url: string) => {
    const target = linkable(url);
    return target ? `[image](${target})` : "image";
  });

  // `<a href="x">y</a>` is how GitHub's own UI writes some references.
  body = body.replace(/<a\b[^>]*?href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi,
    (_all, url: string, label: string) => {
      const text = label.trim() || url;
      const target = linkable(url);
      return target ? `[${text}](${target})` : text;
    });

  // Allowlisted formatting tags become their markdown spelling.
  body = body.replace(/<\/?([a-z]+)\b[^>]*>/gi, (all, rawTag: string) => {
    const equivalent = TAG_EQUIVALENT[rawTag.toLowerCase()];
    return equivalent === undefined ? all : equivalent;
  });

  const origin = repositoryUrl ? accountOrigin(repositoryUrl) : undefined;
  body = body.split("\n").map(line => {
    // Task lists. The renderer has no checkbox input, and a rendered glyph is
    // honest about being a status rather than a control.
    let next = line.replace(/^(\s*[-*+]\s+)\[([ xX])\]\s+/, (_all, bullet: string, mark: string) =>
      `${bullet}${mark === " " ? "☐" : "☑"} `);
    next = autolink(next);
    if (repositoryUrl) {
      next = next.replace(/(^|[\s([])#(\d+)\b/g,
        (_all, lead: string, number: string) => `${lead}[#${number}](${repositoryUrl}/issues/${number})`);
    }
    if (origin) {
      // The leading-boundary group is what keeps `me@example.com` out: a
      // handle only counts at the start of a line or after whitespace.
      next = next.replace(/(^|[\s([])@([A-Za-z\d](?:[A-Za-z\d-]{0,37}[A-Za-z\d])?)\b/g,
        (_all, lead: string, login: string) => `${lead}[@${login}](${origin}/${login})`);
    }
    return next;
  }).join("\n");

  return unmaskInlineCode(body, spans);
}

/**
 * Rewrite GitHub-only markdown into what Bridge's renderer understands.
 *
 * `repositoryUrl` turns `#123` into a real issue link and `@user` into an
 * account link on that repository's own host. It is optional: without it
 * neither becomes a link, because a dead link is worse than plain text.
 */
export function normalizeGithubMarkdown(text: string, repositoryUrl?: string): string {
  const source = text.replaceAll("\r\n", "\n");
  const ranges = fencedRanges(source);
  const out: string[] = [];
  let cursor = 0;
  for (const [start, end] of ranges) {
    if (start > cursor) out.push(rewriteProse(source.slice(cursor, start), repositoryUrl));
    out.push(source.slice(start, end));
    cursor = end;
  }
  if (cursor < source.length) out.push(rewriteProse(source.slice(cursor), repositoryUrl));
  // Collapse the vertical holes stripped comments leave behind.
  return out.join("").replace(/\n{3,}/g, "\n\n").trim();
}

const DETAILS_OPEN = /<details\b[^>]*>/gi;
const DETAILS_CLOSE = "</details>";

/**
 * Split a body into markdown runs and `<details>` sections so each section can
 * render as a real disclosure instead of printing its tags.
 *
 * Markup inside a fenced block is skipped, so a body that *shows* a
 * `<details>` example keeps it as code. Unbalanced markup degrades to one
 * markdown run — a half-open `<details>` is a typo in someone else's comment,
 * not a reason to drop the rest of the body.
 */
export function splitGithubDetails(text: string): GithubBlock[] {
  const ranges = fencedRanges(text);
  const blocks: GithubBlock[] = [];
  const lowered = text.toLowerCase();
  let cursor = 0;
  DETAILS_OPEN.lastIndex = 0;
  for (let open = DETAILS_OPEN.exec(text); open; open = DETAILS_OPEN.exec(text)) {
    const openStart = open.index ?? 0;
    if (openStart < cursor || insideFence(openStart, ranges)) continue;
    let close = lowered.indexOf(DETAILS_CLOSE, openStart + open[0].length);
    while (close !== -1 && insideFence(close, ranges)) {
      close = lowered.indexOf(DETAILS_CLOSE, close + DETAILS_CLOSE.length);
    }
    if (close === -1) break;
    const before = text.slice(cursor, openStart);
    if (before.trim()) blocks.push({ kind: "markdown", text: before });
    const inner = text.slice(openStart + open[0].length, close);
    const summary = /<summary\b[^>]*>([\s\S]*?)<\/summary>/i.exec(inner);
    blocks.push({
      kind: "details",
      summary: (summary?.[1] ?? "Details").replace(/<[^>]+>/g, "").trim() || "Details",
      body: (summary ? inner.slice((summary.index ?? 0) + summary[0].length) : inner).trim(),
    });
    cursor = close + DETAILS_CLOSE.length;
    DETAILS_OPEN.lastIndex = cursor;
  }
  const rest = text.slice(cursor);
  if (rest.trim() || blocks.length === 0) blocks.push({ kind: "markdown", text: rest });
  return blocks;
}
