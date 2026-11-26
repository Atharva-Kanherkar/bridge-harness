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
  /** `open` only appears when the author wrote `<details open>`. */
  | { kind: "details"; summary: string; body: string; open?: true };

/** One image or video a body embeds. */
export type GithubMediaItem = {
  kind: "image" | "video";
  url: string;
  alt: string;
  /** Where a linked image (a badge, a thumbnail) points. */
  href?: string;
  /** The column heading when the media came out of a comparison table. */
  caption?: string;
};

/** A run of markdown, or a row of media that sat between two runs. */
export type GithubMediaBlock =
  | { kind: "markdown"; text: string }
  | { kind: "media"; items: GithubMediaItem[] };

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

// ── Icons and badges ─────────────────────────────────────────────────────────
//
// A status dot, an avatar, a CI badge or a "Fix in Cursor" button is drawn
// inline on GitHub, a few pixels tall, in the middle of a sentence or a table
// cell. Lifting one into a full-width media row (with a "Load from" button
// for a sixteen-pixel avatar) is what made bot comments unreadable, so an
// icon stays in the text and is written as words instead: a badge becomes a
// code chip with what the badge says, any other icon its alt text, and a
// decorative one (no alt) nothing at all.

const BADGE_HOSTS = new Set([
  "img.shields.io", "badgen.net", "badge.fury.io", "dependabot-badges.githubapp.com",
  "api.netlify.com", "app.codacy.com", "api.codeclimate.com", "snyk.io", "deepwiki.com",
]);

function urlParts(url: string): { host: string; path: string } | undefined {
  try {
    const parsed = new URL(url);
    return { host: parsed.hostname.toLowerCase(), path: decodeURIComponent(parsed.pathname) };
  } catch {
    return undefined;
  }
}

/** A status badge: shields.io and friends, or any `badge` path (a GitHub
 *  Actions `badge.svg`, a Codecov graph badge). */
function isBadge(url: string): boolean {
  const parts = urlParts(url);
  if (!parts) return false;
  return BADGE_HOSTS.has(parts.host) || /badges?(?:\.svg)?(?:\/|$)/i.test(parts.path);
}

/** The pixel size an `<img>` tag declares, when it declares one. */
function declaredSize(tag: string | undefined, name: "width" | "height"): number | undefined {
  if (!tag) return undefined;
  const value = attribute(tag, name);
  if (value === undefined || !/^\s*\d+(?:\.\d+)?(?:px)?\s*$/.test(value)) return undefined;
  return Number.parseFloat(value);
}

/** Whether an image is drawn inline rather than shown as a picture. */
function isIcon(url: string, tag?: string): boolean {
  const width = declaredSize(tag, "width");
  const height = declaredSize(tag, "height");
  return isBadge(url) || (width !== undefined && width <= 40) || (height !== undefined && height <= 40);
}

/** What a shields.io badge says: `badge/P1-orange` reads "P1" and
 *  `badge/build-passing-green` reads "build: passing". */
function badgeText(url: string, alt: string): string {
  const parts = urlParts(url);
  const content = parts?.host === "img.shields.io" ? /^\/badge\/([^/]+?)(?:\.svg)?$/.exec(parts.path)?.[1] : undefined;
  if (!content) return alt;
  const pieces = content.replaceAll("--", "\u0000").replaceAll("__", "\u0001").split("-")
    .map(piece => piece.replaceAll("_", " ").replaceAll("\u0000", "-").replaceAll("\u0001", "_").trim());
  if (pieces.length === 2 && pieces[0]) return pieces[0];
  if (pieces.length === 3 && pieces[0] && pieces[1]) return `${pieces[0]}: ${pieces[1]}`;
  return alt;
}

/**
 * An icon written as text. `after` is the text that follows it: Vercel's
 * status column reads `![Ready](ready.svg) [Ready](…)`, and an icon that only
 * repeats the next word says nothing a reader needs twice.
 */
function iconText(url: string, alt: string, after: string): string {
  const label = alt.trim();
  if (isBadge(url)) {
    const text = badgeText(url, label).replaceAll("`", "'");
    return text ? `\`${text}\`` : "";
  }
  if (!label) return "";
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (new RegExp(`^\\s*\\[?${escaped}\\b`, "i").test(after)) return "";
  return label;
}

/** An image that stays in the text: an icon as words, a picture as a link. */
function inlineImage(url: string, alt: string, tag: string | undefined, after: string): string {
  const target = linkable(url);
  if (target && (isIcon(target, tag) || /\.svg$/i.test(urlParts(target)?.path ?? ""))) return iconText(target, alt, after);
  const label = alt.trim() ? `image: ${alt.trim()}` : "image";
  return target ? `[${label}](${target})` : label;
}

// ── HTML GitHub allows, as markdown ──────────────────────────────────────────

/** Tags that only style their text. GitHub keeps the text; so does Bridge. */
const UNWRAP = /<\/?(?:sup|sub|span|small|big|relative-time|time|ins|u|mark|abbr|cite|font|q|var|samp|dfn|bdi|bdo|nobr|picture|center|section|article|figure|figcaption|thead|tbody|tfoot|caption|colgroup|dl)\b[^>]*>/gi;

const NAMED_ENTITIES: Record<string, string> = {
  nbsp: " ", amp: "&", lt: "<", gt: ">", quot: "\"", apos: "'", mdash: "—", ndash: "–", hellip: "…",
  middot: "·", bull: "•", copy: "©", reg: "®", trade: "™", times: "×", rarr: "→", larr: "←", uarr: "↑",
  darr: "↓", harr: "↔", check: "✓", ensp: " ", emsp: " ", thinsp: " ", zwj: "‍",
  zwnj: "‌", lsquo: "‘", rsquo: "’", ldquo: "“", rdquo: "”", laquo: "«", raquo: "»", deg: "°",
  plusmn: "±", ne: "≠", le: "≤", ge: "≥", infin: "∞", sect: "§", para: "¶", dagger: "†", star: "☆",
};

/** Decode the character references GitHub decodes. The result only ever
 *  reaches the DOM as a React text node, so a decoded `<` stays text. */
export function decodeEntities(text: string): string {
  return text.replace(/&(#\d{1,7}|#x[\da-f]{1,6}|[a-z][a-z\d]{1,31});/gi, (all, name: string) => {
    if (name[0] === "#") {
      const code = name[1] === "x" || name[1] === "X" ? Number.parseInt(name.slice(2), 16) : Number.parseInt(name.slice(1), 10);
      return code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff) ? String.fromCodePoint(code) : all;
    }
    return NAMED_ENTITIES[name.toLowerCase()] ?? all;
  });
}

/** The shortcodes bots and templates actually write. GitHub knows ~1,800;
 *  an unknown one stays as typed, which is also what GitHub does. */
const EMOJI: Record<string, string> = {
  white_check_mark: "✅", heavy_check_mark: "✔️", ballot_box_with_check: "☑️", x: "❌", negative_squared_cross_mark: "❎",
  warning: "⚠️", exclamation: "❗", question: "❓", information_source: "ℹ️", bulb: "💡", memo: "📝", pencil: "📝",
  pencil2: "✏️", rocket: "🚀", tada: "🎉", sparkles: "✨", fire: "🔥", bug: "🐛", eyes: "👀", "+1": "👍", "-1": "👎",
  thumbsup: "👍", thumbsdown: "👎", heart: "❤️", star: "⭐", zap: "⚡", boom: "💥", recycle: "♻️", wrench: "🔧",
  hammer: "🔨", hammer_and_wrench: "🛠️", gear: "⚙️", package: "📦", lock: "🔒", unlock: "🔓", key: "🔑", link: "🔗",
  mag: "🔍", clipboard: "📋", construction: "🚧", rotating_light: "🚨", stop_sign: "🛑", no_entry: "⛔",
  no_entry_sign: "🚫", hourglass: "⌛", hourglass_flowing_sand: "⏳", stopwatch: "⏱️", alarm_clock: "⏰", dart: "🎯",
  robot: "🤖", test_tube: "🧪", microscope: "🔬", chart_with_upwards_trend: "📈", chart_with_downwards_trend: "📉",
  bar_chart: "📊", loudspeaker: "📢", mega: "📣", bell: "🔔", speech_balloon: "💬", thought_balloon: "💭",
  arrow_up: "⬆️", arrow_down: "⬇️", arrow_right: "➡️", arrow_left: "⬅️", arrow_forward: "▶️", heavy_plus_sign: "➕",
  heavy_minus_sign: "➖", red_circle: "🔴", orange_circle: "🟠", yellow_circle: "🟡", green_circle: "🟢",
  large_blue_circle: "🔵", blue_circle: "🔵", purple_circle: "🟣", white_circle: "⚪", black_circle: "⚫",
  green_heart: "💚", broken_heart: "💔", umbrella: "☂️", shield: "🛡️", lipstick: "💄", art: "🎨", books: "📚",
  book: "📖", page_facing_up: "📄", file_folder: "📁", open_file_folder: "📂", calendar: "📅", pushpin: "📌",
  paperclip: "📎", label: "🏷️", trophy: "🏆", medal_sports: "🏅", party_popper: "🎉", wave: "👋", pray: "🙏",
  clap: "👏", muscle: "💪", raised_hands: "🙌", ok_hand: "👌", point_right: "👉", point_left: "👈", thinking: "🤔",
  smile: "😄", smiley: "😃", grin: "😁", joy: "😂", sweat_smile: "😅", wink: "😉", confused: "😕", cry: "😢",
  skull: "💀", ghost: "👻", see_no_evil: "🙈", rabbit: "🐰", rabbit2: "🐇", penguin: "🐧", whale: "🐳", seedling: "🌱",
  globe_with_meridians: "🌐", computer: "💻", iphone: "📱", floppy_disk: "💾", truck: "🚚", ambulance: "🚑",
  heavy_exclamation_mark: "❗", grey_exclamation: "❕", grey_question: "❔", new: "🆕", up: "🆙", free: "🆓",
};

const SHORTCODE = /(^|[\s(>|*_~]):([a-z\d_+-]{1,40}):(?=$|[\s).,;!?<|*_~])/g;

function replaceShortcodes(line: string): string {
  // Twice, so neighbours that share a space (`:a: :b:`) both match.
  const once = (text: string) => text.replace(SHORTCODE, (all, lead: string, name: string) => {
    const emoji = EMOJI[name];
    return emoji === undefined ? all : `${lead}${emoji}`;
  });
  return once(once(line));
}

/** A table row in markdown: a line that starts and ends with a pipe. A line
 *  break inside one would split the row, so `<br>` there is a space. */
const MARKDOWN_TABLE_ROW = /^\s*\|.*\|\s*$/;

/** Turn `<ul>`/`<ol>`/`<li>` into markdown list lines, nested by depth. */
function htmlLists(body: string): string {
  const stack: Array<"ul" | "ol"> = [];
  return body.replace(/<(\/?)(ul|ol|li)\b[^>]*>\s*/gi, (_all, closing: string, tag: string) => {
    const name = tag.toLowerCase();
    if (name === "li") {
      if (closing) return "";
      const indent = "  ".repeat(Math.max(0, stack.length - 1));
      return `\n${indent}${stack[stack.length - 1] === "ol" ? "1." : "-"} `;
    }
    // Only the outermost list is set apart; a nested one hangs off its item.
    if (closing) { stack.pop(); return stack.length ? "" : "\n\n"; }
    stack.push(name as "ul" | "ol");
    return stack.length === 1 ? "\n\n" : "";
  });
}

/** A simple HTML table as a markdown one. Cells are inline text; a pipe in a
 *  cell would end it, so it is drawn as a lookalike. */
function htmlTables(body: string): string {
  return body.replace(/<table\b[^>]*>([\s\S]*?)<\/table>/gi, (all, inner: string) => {
    const rows = [...inner.matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)].map(row =>
      [...row[1].matchAll(/<t([hd])\b[^>]*>([\s\S]*?)<\/t\1>/gi)].map(cell =>
        cell[2].replace(/<\/?([a-z]+)\b[^>]*>/gi, (_all, tag: string) => TAG_EQUIVALENT[tag.toLowerCase()] ?? " ")
          .replace(/\s+/g, " ").replaceAll("|", "∣").trim()));
    const width = Math.max(0, ...rows.map(row => row.length));
    if (!rows.length || !width) return all;
    const line = (cells: string[]) => `| ${Array.from({ length: width }, (_, index) => cells[index] ?? "").join(" | ")} |`;
    return `\n\n${line(rows[0])}\n|${" --- |".repeat(width)}\n${rows.slice(1).map(line).join("\n")}\n\n`;
  });
}

/** `<blockquote>` as `>` lines, innermost first so nesting survives. */
function htmlQuotes(body: string): string {
  let out = body;
  for (let pass = 0; pass < 4 && /<blockquote\b/i.test(out); pass += 1) {
    out = out.replace(/<blockquote\b[^>]*>((?:(?!<blockquote\b)[\s\S])*?)<\/blockquote>/gi, (_all, inner: string) => {
      const lines = inner.replace(/\n{3,}/g, "\n\n").trim().split("\n");
      return `\n\n${lines.map(line => line.trim() ? `> ${line.trim()}` : ">").join("\n")}\n\n`;
    });
  }
  return out;
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

  // `<br>` is the one tag whose meaning is purely a line break, except inside
  // a markdown table row, where a break would split the row in two.
  body = body.split("\n").map(line => line.replace(/<br\s*\/?>/gi, MARKDOWN_TABLE_ROW.test(line) ? " " : "\n")).join("\n");

  // `<picture>` only chooses between sources; its `<img>` is the fallback
  // GitHub draws, and the one kept here.
  body = body.replace(/<source\b[^>]*>|<wbr\s*\/?>/gi, "");

  // An image left in the text. Bridge does not fetch remote images into the
  // text: a comment body is attacker-influenced, and an `<img src>` is a
  // network request (and a read receipt) we never promised. Pictures that
  // stand on their own were lifted out as media before this runs (see
  // `splitGithubMedia`); what remains is an icon, written as words, or a
  // picture inside prose, written as a labelled link.
  body = body.replace(/!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/g, (all, alt: string, url: string, offset: number, whole: string) =>
    inlineImage(url, alt, undefined, whole.slice(offset + all.length)));
  // Quoted attributes are read whole: a `src` may itself contain a `>`.
  body = body.replace(/<img\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi, (tag: string, offset: number, whole: string) => {
    const url = attribute(tag, "src");
    if (url === undefined) return "";
    return inlineImage(url, attribute(tag, "alt") ?? "", tag, whole.slice(offset + tag.length));
  });

  // Styling-only wrappers give way to their text, and a key is a code chip.
  body = body.replace(UNWRAP, "");
  body = body.replace(/<\/?kbd\b[^>]*>/gi, "`");

  // `<a href="x">y</a>` is how GitHub's own UI writes some references. One
  // left with no label (it only wrapped an avatar) disappears.
  body = body.replace(/<a\b[^>]*?href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi,
    (_all, url: string, label: string) => {
      const text = label.replace(/\s+/g, " ").trim();
      if (!text) return "";
      const target = linkable(url);
      return target ? `[${text}](${target})` : text;
    });
  // A named anchor (`<a name="install"></a>`) marks a place, not a link.
  body = body.replace(/<\/?a\b[^>]*>/gi, "");

  // Block markup, as the markdown that draws the same thing.
  body = body.replace(/<h([1-6])\b[^>]*>([\s\S]*?)<\/h\1>/gi, (_all, level: string, text: string) =>
    `\n\n${"#".repeat(Number(level))} ${text.replace(/\s+/g, " ").trim()}\n\n`);
  body = body.replace(/<hr\b[^>]*>/gi, "\n\n---\n\n");
  body = htmlTables(body);
  body = htmlLists(body);
  body = body.replace(/<\/?(?:p|div)\b[^>]*>/gi, "\n\n");
  body = body.replace(/<dt\b[^>]*>([\s\S]*?)<\/dt>/gi, (_all, text: string) => `\n\n**${text.trim()}**\n`);
  body = body.replace(/<\/?dd\b[^>]*>/gi, "\n");
  // Whatever table structure survived (a table split around a screenshot)
  // reads as rows of words rather than as tags.
  body = body.replace(/<\/?(?:table|tr)\b[^>]*>/gi, "\n").replace(/<\/?t[hd]\b[^>]*>/gi, " ");
  body = htmlQuotes(body);

  // Allowlisted formatting tags become their markdown spelling.
  body = body.replace(/<\/?([a-z]+)\b[^>]*>/gi, (all, rawTag: string) => {
    const equivalent = TAG_EQUIVALENT[rawTag.toLowerCase()];
    return equivalent === undefined ? all : equivalent;
  });

  const origin = repositoryUrl ? accountOrigin(repositoryUrl) : undefined;
  body = body.split("\n").map(line => {
    // Task lists. The renderer has no checkbox input, and a rendered glyph is
    // honest about being a status rather than a control.
    const listed = line.replace(/^(\s*[-*+]\s+)\[([ xX])\]\s+/, (_all, bullet: string, mark: string) =>
      `${bullet}${mark === " " ? "☐" : "☑"} `);
    // References only link outside an existing link: `[#12](…)`, which is
    // what an `<a>` around `#12` became above, must not nest another link.
    return listed.split(/(\[[^\]\n]*\]\([^)\s]*\))/).map((part, index) => index % 2 ? part : linkReferences(part)).join("");
  }).join("\n");

  // Last, so a decoded `&lt;b&gt;` is text rather than a tag to rewrite.
  return unmaskInlineCode(decodeEntities(body), spans);

  function linkReferences(text: string): string {
    let next = autolink(text);
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
    return replaceShortcodes(next);
  }
}

// ── Reference links and footnotes ────────────────────────────────────────────
//
// `[text][docs]` with `[docs]: https://…` further down is ordinary markdown,
// and a definition line never shows on GitHub. Bots lean on that: Vercel opens
// every comment with `[vc]: #<base64 state>`, which printed as a line of noise
// running off the edge of the card. Definitions are read from the whole body,
// so they resolve no matter which section or media split a use lands in.

const REFERENCE_DEFINITION = /^ {0,3}\[([^\]\n]+)\]:[ \t]*<?([^\s>]+)>?(?:[ \t]+(?:"[^"\n]*"|'[^'\n]*'|\([^)\n]*\)))?[ \t]*$/;
const FOOTNOTE_DEFINITION = /^ {0,3}\[\^([^\]\s]+)\]:[ \t]*(.*)$/;
const SUPERSCRIPT_DIGITS = "⁰¹²³⁴⁵⁶⁷⁸⁹";

const referenceKey = (label: string) => label.trim().replace(/\s+/g, " ").toLowerCase();
const footnoteMark = (id: string) => /^\d+$/.test(id) ? [...id].map(digit => SUPERSCRIPT_DIGITS[Number(digit)]).join("") : `[${id}]`;

/** Resolve reference-style links and footnotes, and drop their definitions. */
export function resolveReferences(text: string): string {
  const source = text.replaceAll("\r\n", "\n");
  if (!/^ {0,3}\[[^\]\n]+\]:/m.test(source)) return text;
  const definitionRanges = fencedRanges(source);
  const definitions = new Map<string, string>();
  const kept: string[] = [];
  let offset = 0;
  for (const line of source.split("\n")) {
    const at = offset;
    offset += line.length + 1;
    if (!insideFence(at, definitionRanges)) {
      const footnote = FOOTNOTE_DEFINITION.exec(line);
      if (footnote) { kept.push(`${footnoteMark(footnote[1])} ${footnote[2]}`); continue; }
      const definition = REFERENCE_DEFINITION.exec(line);
      if (definition) {
        const key = referenceKey(definition[1]);
        // The first definition of a label wins, as in CommonMark.
        if (!definitions.has(key)) definitions.set(key, definition[2]);
        continue;
      }
    }
    kept.push(line);
  }
  const body = kept.join("\n");
  const link = (bang: string, label: string, key: string, all: string) => {
    const url = definitions.get(referenceKey(key));
    if (url === undefined) return all;
    const target = linkable(url);
    return target ? `${bang}[${label}](${target})` : label;
  };
  const resolve = (prose: string) => {
    const { masked, spans } = maskInlineCode(prose);
    const out = masked
      .replace(/(!?)\[([^\]\n]+)\]\[([^\]\n]*)\]/g, (all, bang: string, label: string, key: string) =>
        label.startsWith("^") ? all : link(bang, label, key || label, all))
      .replace(/\[\^([^\]\s]+)\](?!:)/g, (_all, id: string) => footnoteMark(id))
      .replace(/(!?)\[([^\]\n]+)\](?![([:])/g, (all, bang: string, label: string) => link(bang, label, label, all));
    return unmaskInlineCode(out, spans);
  };
  const ranges = fencedRanges(body);
  const out: string[] = [];
  let cursor = 0;
  for (const [start, end] of ranges) {
    if (start > cursor) out.push(resolve(body.slice(cursor, start)));
    out.push(body.slice(start, end));
    cursor = end;
  }
  if (cursor < body.length) out.push(resolve(body.slice(cursor)));
  return out.join("");
}

/**
 * Rewrite GitHub-only markdown into what Bridge's renderer understands.
 *
 * `repositoryUrl` turns `#123` into a real issue link and `@user` into an
 * account link on that repository's own host. It is optional: without it
 * neither becomes a link, because a dead link is worse than plain text.
 */
export function normalizeGithubMarkdown(text: string, repositoryUrl?: string): string {
  const source = resolveReferences(text).replaceAll("\r\n", "\n");
  const ranges = fencedRanges(source);
  const out: string[] = [];
  // Collapse the vertical holes stripped comments and block tags leave
  // behind, in prose only: blank lines inside a fence are the author's.
  const prose = (chunk: string) => rewriteProse(chunk, repositoryUrl).replace(/\n[ \t]*(?:\n[ \t]*){2,}/g, "\n\n");
  let cursor = 0;
  for (const [start, end] of ranges) {
    if (start > cursor) out.push(prose(source.slice(cursor, start)));
    out.push(source.slice(start, end));
    cursor = end;
  }
  if (cursor < source.length) out.push(prose(source.slice(cursor)));
  return out.join("").trim();
}

const DETAILS_TAG = /<details\b[^>]*>|<\/details>/gi;

/** Where the `</details>` that closes the section opened at `from` starts,
 *  counting nested sections and skipping fenced code; -1 when unclosed. */
function matchingClose(text: string, from: number, ranges: Array<[number, number]>): number {
  let depth = 1;
  DETAILS_TAG.lastIndex = from;
  for (let tag = DETAILS_TAG.exec(text); tag; tag = DETAILS_TAG.exec(text)) {
    if (insideFence(tag.index, ranges)) continue;
    depth += tag[0][1] === "/" ? -1 : 1;
    if (depth === 0) return tag.index;
  }
  return -1;
}

/**
 * Split a body into markdown runs and `<details>` sections so each section can
 * render as a real disclosure instead of printing its tags.
 *
 * A section's body may hold sections of its own (CodeRabbit nests three
 * deep); the outer one owns everything up to its matching close, and the
 * renderer splits the body again. Markup inside a fenced block is skipped, so
 * a body that *shows* a `<details>` example keeps it as code. Unbalanced
 * markup degrades to one markdown run — a half-open `<details>` is a typo in
 * someone else's comment, not a reason to drop the rest of the body.
 */
export function splitGithubDetails(text: string): GithubBlock[] {
  const ranges = fencedRanges(text);
  const blocks: GithubBlock[] = [];
  const opener = /<details\b[^>]*>/gi;
  let cursor = 0;
  for (let open = opener.exec(text); open; open = opener.exec(text)) {
    const openStart = open.index;
    if (openStart < cursor || insideFence(openStart, ranges)) continue;
    const close = matchingClose(text, openStart + open[0].length, ranges);
    if (close === -1) break;
    const before = text.slice(cursor, openStart);
    if (before.trim()) blocks.push({ kind: "markdown", text: before });
    const inner = text.slice(openStart + open[0].length, close);
    // The section's own summary: the first one not inside a nested section.
    const summary = /^\s*<summary\b[^>]*>([\s\S]*?)<\/summary>/i.exec(inner);
    blocks.push({
      kind: "details",
      summary: decodeEntities((summary?.[1] ?? "Details").replace(/<[^>]+>/g, "")).replace(/\s+/g, " ").trim() || "Details",
      body: (summary ? inner.slice(summary.index + summary[0].length) : inner).trim(),
      ...(/\sopen\b/i.test(open[0]) ? { open: true as const } : {}),
    });
    cursor = close + "</details>".length;
    opener.lastIndex = cursor;
  }
  const rest = text.slice(cursor);
  if (rest.trim() || blocks.length === 0) blocks.push({ kind: "markdown", text: rest });
  return blocks;
}


// ── Media ────────────────────────────────────────────────────────────────────
//
// A pull request is reviewed through its screenshots and recordings as much as
// its diff, so the pane shows them. They are lifted out of the markdown before
// the text renderer runs: the renderer stays a text renderer, and a piece of
// media becomes a typed item the pane decides how to load (see
// `github/Media.tsx`, which is also where the "which hosts load on their own"
// rule lives). Only an http(s) address is ever lifted; anything else stays in
// the text and takes the inert path above.

const VIDEO_EXTENSION = /\.(?:mp4|mov|webm|m4v)$/i;
/** GitHub's own attachment addresses, which carry no file extension. */
const ATTACHMENT_PATH = /^\/(?:user-attachments\/assets\/[\w-]+|[^/]+\/[^/]+\/assets\/\d+\/[\w-]+)\/?$/;

function mediaKind(url: string): "image" | "video" {
  try {
    return VIDEO_EXTENSION.test(new URL(url).pathname) ? "video" : "image";
  } catch {
    return "image";
  }
}

/** A URL that stands alone on a line and is a recording: GitHub writes an
 *  uploaded video into the body as exactly that. */
function bareVideo(url: string): boolean {
  try {
    const parsed = new URL(url);
    if (VIDEO_EXTENSION.test(parsed.pathname)) return true;
    return /^https:$/.test(parsed.protocol) && !parsed.hostname.includes("githubusercontent") && ATTACHMENT_PATH.test(parsed.pathname);
  } catch {
    return false;
  }
}

const attribute = (tag: string, name: string): string | undefined =>
  new RegExp(`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, "i").exec(tag)?.slice(1).find(value => value !== undefined);

// Alternatives, in the order they must be tried: a linked image before a bare
// one, an `<a><img></a>` before a bare `<img>`.
const MEDIA_TOKEN = new RegExp([
  String.raw`\[!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)\]\(([^)\s]+)\)`,
  String.raw`!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)`,
  // A linked image, through the wrappers bots put around one: Vercel's
  // `<a><sup><img></sup></a>` avatar, Cursor's `<a><picture>…<img></picture></a>`.
  String.raw`<a\b[^>]*>\s*(?:<(?:picture|sup|sub|span|p)\b[^>]*>\s*|<source\b[^>]*>\s*)*<img\b[^>]*>\s*(?:<\/(?:picture|sup|sub|span|p)>\s*)*<\/a>`,
  String.raw`<video\b[^>]*>(?:[\s\S]*?<\/video>)?`,
  String.raw`<img\b[^>]*>`,
].join("|"), "gi");

/** Wrapper markup that carries no content of its own once its media is gone. */
const EMPTY_WRAPPER = /<\/?(?:p|div|center|picture|span|kbd)\b[^>]*>|<source\b[^>]*>|<br\s*\/?>/gi;

function mediaFromToken(match: RegExpExecArray | RegExpMatchArray): GithubMediaItem | undefined {
  const token = match[0];
  let url: string | undefined;
  let alt = "";
  let href: string | undefined;
  if (match[2] !== undefined) { alt = match[1] ?? ""; url = match[2]; href = match[3]; }
  else if (match[5] !== undefined) { alt = match[4] ?? ""; url = match[5]; }
  else if (/^<a\b/i.test(token)) {
    const image = /<img\b[^>]*>/i.exec(token)?.[0] ?? "";
    url = attribute(image, "src"); alt = attribute(image, "alt") ?? ""; href = attribute(token.slice(0, token.indexOf(">") + 1), "href");
  } else {
    url = attribute(token, "src"); alt = attribute(token, "alt") ?? attribute(token, "title") ?? "";
  }
  const target = url === undefined ? undefined : linkable(url);
  if (!target) return undefined;
  const link = href === undefined ? undefined : linkable(href);
  const video = /^<video\b/i.test(token);
  return { kind: video ? "video" : mediaKind(target), url: target, alt: alt.trim(), ...(link && !video ? { href: link } : {}) };
}

/** Whether a media token is an icon, which stays in the text (see `isIcon`). */
function iconToken(token: string, item: GithubMediaItem): boolean {
  if (item.kind === "video") return false;
  return isIcon(item.url, /<img\b[^>]*>/i.exec(token)?.[0]);
}

/** The media on one line, and what is left of the line without it. */
function liftLine(line: string): { items: GithubMediaItem[]; rest: string } {
  const { masked, spans } = maskInlineCode(line);
  const items: GithubMediaItem[] = [];
  let rest = masked.replace(MEDIA_TOKEN, (...args) => {
    const item = mediaFromToken(args as unknown as RegExpMatchArray);
    if (!item || iconToken(args[0] as string, item)) return args[0] as string;
    items.push(item);
    return "";
  });
  if (!items.length) {
    const bare = /^\s*<?(https?:\/\/[^\s<>]+)>?\s*$/.exec(masked);
    if (bare && bareVideo(bare[1])) return { items: [{ kind: "video", url: bare[1], alt: "" }], rest: "" };
    return { items, rest: line };
  }
  if (!rest.replace(EMPTY_WRAPPER, "").replace(/[|\s]/g, "")) rest = "";
  return { items, rest: unmaskInlineCode(rest, spans) };
}

const TABLE_SEPARATOR = /^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)*\|?\s*$/;
const tableCells = (line: string): string[] => line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(cell => cell.trim());

/**
 * A "Dark | Light" comparison table whose every body cell is a screenshot.
 * It becomes rows of media captioned by the column heading, because a table
 * cell is a line of text in the renderer and cannot hold an image. A table
 * with any prose in its body is left alone.
 */
function liftTable(lines: string[]): GithubMediaItem[][] | undefined {
  const header = tableCells(lines[0]);
  const rows: GithubMediaItem[][] = [];
  for (const line of lines.slice(2)) {
    const row: GithubMediaItem[] = [];
    for (const [column, cell] of tableCells(line).entries()) {
      if (!cell) continue;
      const lifted = liftLine(cell);
      if (lifted.rest || lifted.items.length !== 1) return undefined;
      const caption = header[column]?.replace(/[*_`]/g, "").trim();
      row.push({ ...lifted.items[0], ...(caption ? { caption } : {}) });
    }
    if (row.length) rows.push(row);
  }
  return rows.length ? rows : undefined;
}

/**
 * Split a run of GitHub markdown into text and the media between it.
 *
 * Fenced code is never searched, so a body that *shows* an `<img>` keeps it as
 * code. Consecutive media lines share one row, which is how a pair of
 * before/after screenshots reads side by side.
 */
export function splitGithubMedia(text: string): GithubMediaBlock[] {
  // A tag may be wrapped across lines; the split below is line-based.
  const source = text.replaceAll("\r\n", "\n");
  const ranges = fencedRanges(source);
  const flattened = source.replace(/<(?:img|video|a)\b[^>]*>/gi, (tag, offset: number) =>
    insideFence(offset, ranges) ? tag : tag.replace(/\s*\n\s*/g, " "));
  const fences = fencedRanges(flattened);
  const blocks: GithubMediaBlock[] = [];
  let prose: string[] = [];
  const flushProse = () => {
    if (prose.join("\n").trim()) blocks.push({ kind: "markdown", text: prose.join("\n") });
    prose = [];
  };
  const pushMedia = (items: GithubMediaItem[], join: boolean) => {
    const last = blocks[blocks.length - 1];
    if (join && !prose.length && last?.kind === "media") last.items.push(...items);
    else { flushProse(); blocks.push({ kind: "media", items: [...items] }); }
  };

  const lines = flattened.split("\n");
  let offset = 0;
  const offsets = lines.map(line => { const at = offset; offset += line.length + 1; return at; });
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (insideFence(offsets[index], fences)) { prose.push(line); continue; }
    if (line.includes("|") && TABLE_SEPARATOR.test(lines[index + 1] ?? "") && (lines[index + 1] ?? "").includes("-")) {
      let end = index + 2;
      while (end < lines.length && lines[end].includes("|") && lines[end].trim() && !insideFence(offsets[end], fences)) end += 1;
      const rows = liftTable(lines.slice(index, end));
      if (rows) {
        for (const row of rows) pushMedia(row, false);
      } else {
        // A table of prose keeps its pictures in their cells. Lifting one
        // out tore the row apart and left `<sup></sup>` where it had been.
        prose.push(...lines.slice(index, end));
      }
      index = end - 1;
      continue;
    }
    const lifted = liftLine(line);
    if (!lifted.items.length) { prose.push(line); continue; }
    if (lifted.rest.trim()) { prose.push(lifted.rest); pushMedia(lifted.items, false); }
    else pushMedia(lifted.items, true);
  }
  flushProse();
  return blocks.length ? blocks : [{ kind: "markdown", text }];
}
