// GitHub-authored markdown is not the markdown Bridge's renderer was written
// for. A PR body arrives with `<details>` sections, HTML comments left behind
// by a template, `<img>` screenshots, `- [x]` task lists, bare URLs, and
// `#123`/`@user` cross-references — and the renderer printed every one of them
// literally, which is what "markdown doesn't render" actually looked like.
//
// This module is the translation layer, and it is deliberately *lossy in the
// safe direction*. Remote markup is never handed to the DOM: an allowlisted
// formatting tag is rewritten into its markdown equivalent, an image becomes a
// labelled link rather than a network fetch, and everything else — `<script>`
// above all — is left exactly as authored so it renders as inert text through
// React the way it always has.

/** One run of a GitHub body: ordinary markdown, or a collapsible section. */
export type GithubBlock =
  | { kind: "markdown"; text: string }
  | { kind: "details"; summary: string; body: string };

/** Formatting tags GitHub allows in a comment that carry a markdown spelling.
 * Anything absent from this table stays verbatim text. */
const TAG_EQUIVALENT: Record<string, string> = {
  b: "**", strong: "**", i: "*", em: "*", code: "`", del: "~~", s: "~~", strike: "~~",
};

const FENCE = /^ {0,3}(?:```|~~~)/;

/** Split a body into fenced-code runs and prose runs. Only prose is rewritten;
 * a fenced block is content, not markup, and rewriting inside it would corrupt
 * the very diffs and snippets reviewers paste. */
function proseRuns(text: string): Array<{ code: boolean; lines: string[] }> {
  const runs: Array<{ code: boolean; lines: string[] }> = [];
  let current: { code: boolean; lines: string[] } = { code: false, lines: [] };
  let fenced = false;
  for (const line of text.split("\n")) {
    if (FENCE.test(line.trim())) {
      runs.push(current);
      current = { code: !fenced, lines: [line] };
      if (!fenced) { fenced = true; continue; }
      fenced = false;
      runs.push(current);
      current = { code: false, lines: [] };
      continue;
    }
    current.lines.push(line);
  }
  runs.push(current);
  return runs.filter(run => run.lines.length > 0);
}

/** Apply `transform` to the parts of a line that are not inside an inline code
 * span, so a backticked `#4` or `@handle` is left as the author wrote it. */
function outsideCode(line: string, transform: (part: string) => string): string {
  return line
    .split(/(`[^`]*`)/)
    .map(part => (part.startsWith("`") && part.endsWith("`") && part.length > 1 ? part : transform(part)))
    .join("");
}

/** A URL that is already the target of a markdown link or an autolink. */
const LINKED_URL = /(\]\([^)\s]*$|<[^>\s]*$)/;

const BARE_URL = /https?:\/\/[^\s<>()[\]"'`]+[^\s<>()[\]"'`.,;:!?]/g;

function autolink(part: string): string {
  let out = "";
  let last = 0;
  for (const match of part.matchAll(BARE_URL)) {
    const start = match.index ?? 0;
    const before = part.slice(last, start);
    out += before;
    // Skip a URL that is already inside `](…)` or `<…>` — relinking it would
    // produce `[[url](url)](url)`.
    out += LINKED_URL.test(out) ? match[0] : `[${match[0]}](${match[0]})`;
    last = start + match[0].length;
  }
  return out + part.slice(last);
}

/**
 * Rewrite GitHub-only markdown into what Bridge's renderer understands.
 *
 * `repositoryUrl` turns `#123` into a real issue link and is optional: without
 * it the reference stays plain text rather than becoming a dead link.
 */
export function normalizeGithubMarkdown(text: string, repositoryUrl?: string): string {
  const runs = proseRuns(text.replaceAll("\r\n", "\n"));
  const rewritten = runs.map(run => {
    if (run.code) return run.lines.join("\n");
    let body = run.lines.join("\n");

    // Template leftovers. An HTML comment is invisible on GitHub, so showing
    // it is strictly wrong rather than merely unstyled. A comment that owns
    // its whole line takes the line with it, or every stripped instruction
    // block would leave a gap where prose used to join up.
    body = body.replace(/^[ \t]*<!--[\s\S]*?-->[ \t]*\n?/gm, "");
    body = body.replace(/<!--[\s\S]*?-->/g, "");

    // `<br>` is the one tag whose meaning is purely a line break.
    body = body.replace(/<br\s*\/?>/gi, "\n");

    // A screenshot becomes a labelled link. Bridge does not fetch remote
    // images into the pane: a comment body is attacker-influenced, and an
    // `<img src>` is a network request (and a read receipt) we never promised.
    body = body.replace(/!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/g, (_all, alt: string, url: string) =>
      `[${alt.trim() ? `image: ${alt.trim()}` : "image"}](${url})`);
    body = body.replace(/<img\b[^>]*?src=["']([^"']+)["'][^>]*>/gi, (_all, url: string) => `[image](${url})`);

    // `<a href="x">y</a>` is how GitHub's own UI writes some references.
    body = body.replace(/<a\b[^>]*?href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi,
      (_all, url: string, label: string) => `[${label.trim() || url}](${url})`);

    // Allowlisted formatting tags become their markdown spelling.
    body = body.replace(/<\/?([a-z]+)\b[^>]*>/gi, (all, rawTag: string) => {
      const equivalent = TAG_EQUIVALENT[rawTag.toLowerCase()];
      return equivalent === undefined ? all : equivalent;
    });

    body = body.split("\n").map(line => {
      // Task lists. The renderer has no checkbox input, and a rendered glyph
      // is honest about being a status rather than a control.
      let next = line.replace(/^(\s*[-*+]\s+)\[([ xX])\]\s+/, (_all, bullet: string, mark: string) =>
        `${bullet}${mark === " " ? "☐" : "☑"} `);
      next = outsideCode(next, autolink);
      if (repositoryUrl) {
        next = outsideCode(next, part => part.replace(/(^|[\s([])#(\d+)\b/g,
          (_all, lead: string, number: string) => `${lead}[#${number}](${repositoryUrl}/issues/${number})`));
      }
      // The leading-boundary group is what keeps `me@example.com` out: a
      // handle only counts at the start of a line or after whitespace.
      next = outsideCode(next, part => part.replace(/(^|[\s([])@([A-Za-z\d](?:[A-Za-z\d-]{0,37}[A-Za-z\d])?)\b/g,
        (_all, lead: string, login: string) => `${lead}[@${login}](https://github.com/${login})`));
      return next;
    }).join("\n");

    return body;
  });

  // Collapse the vertical holes stripped comments leave behind.
  return rewritten.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

const DETAILS_OPEN = /<details\b[^>]*>/i;

/**
 * Split a body into markdown runs and `<details>` sections so each section can
 * render as a real disclosure instead of printing its tags. Unbalanced markup
 * degrades to one markdown run — a half-open `<details>` is a typo in someone
 * else's comment, not a reason to drop the rest of the body.
 */
export function splitGithubDetails(text: string): GithubBlock[] {
  const blocks: GithubBlock[] = [];
  let rest = text;
  for (;;) {
    const open = DETAILS_OPEN.exec(rest);
    if (!open) break;
    const openEnd = (open.index ?? 0) + open[0].length;
    const close = rest.toLowerCase().indexOf("</details>", openEnd);
    if (close === -1) break;
    const before = rest.slice(0, open.index);
    if (before.trim()) blocks.push({ kind: "markdown", text: before });
    const inner = rest.slice(openEnd, close);
    const summary = /<summary\b[^>]*>([\s\S]*?)<\/summary>/i.exec(inner);
    blocks.push({
      kind: "details",
      summary: (summary?.[1] ?? "Details").replace(/<[^>]+>/g, "").trim() || "Details",
      body: (summary ? inner.slice((summary.index ?? 0) + summary[0].length) : inner).trim(),
    });
    rest = rest.slice(close + "</details>".length);
  }
  if (rest.trim() || blocks.length === 0) blocks.push({ kind: "markdown", text: rest });
  return blocks;
}
