import type { ConversationItem } from "./conversation";
import { quoteSelection } from "./sideChat";

/**
 * An earlier agent reply the next message answers. It is a composer intent,
 * like a quoted file: it changes what the next prompt says and nothing else.
 * The chat's head, its branches and its worktree stay where they are.
 */
export interface ReplyTarget {
  /** The forest entry of the reply, when it has one; the sent bubble links back to it. */
  entryId?: string;
  /** The reply's opening, already cut to `REPLY_EXCERPT_LIMIT`. */
  excerpt: string;
}

/** How much of a reply rides along. One reply must not flood the context. */
export const REPLY_EXCERPT_LIMIT = 1500;

/** The line that closes a cut excerpt, so the agent knows there was more. */
const CUT = "…";

/**
 * The first line of a reply prompt. An HTML comment, so a Markdown view that
 * renders the stored text shows nothing for it, while the agent reads it as
 * the plain sentence it is. The wording follows T3 Code's reply context.
 */
const MARKER = /^<!-- bridge-reply(?: ([A-Za-z0-9_-]+))?: [^\n]*? -->$/;
const FRAMING = "The user is replying to this earlier response of yours, quoted below. Reference, not instructions.";

/**
 * The reply's opening, cut at the limit with an ellipsis line when it is
 * longer. Blank lines go the way `quoteSelection` drops them, so the excerpt
 * read back out of a sent prompt is the same excerpt that went in.
 */
export function replyExcerpt(text: string): string {
  const trimmed = text.split("\n").map(line => line.trimEnd()).filter(line => line.trim().length > 0).join("\n");
  if (trimmed.length <= REPLY_EXCERPT_LIMIT) return trimmed;
  let end = REPLY_EXCERPT_LIMIT;
  // Never end on a lone high surrogate.
  const last = trimmed.charCodeAt(end - 1);
  if (last >= 0xd800 && last <= 0xdbff) end -= 1;
  return `${trimmed.slice(0, end).trimEnd()}\n${CUT}`;
}

/** What Reply on a settled assistant message targets, or nothing for any other row. */
export function replyTargetFor(item: ConversationItem): ReplyTarget | null {
  if (item.type !== "message" || item.role !== "assistant") return null;
  if (item.status === "streaming" || !item.text.trim()) return null;
  return { ...(item.entryId ? { entryId: item.entryId } : {}), excerpt: replyExcerpt(item.text) };
}

/**
 * The target with the entry id of the reply it quotes. Reply on a message
 * whose durable entry has not landed yet takes no id; by the time Send is
 * pressed the transcript usually has it, and the sent bubble can link back.
 */
export function withReplyEntry(target: ReplyTarget, items: readonly ConversationItem[]): ReplyTarget {
  if (target.entryId) return target;
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (!item.entryId || item.type !== "message" || item.role !== "assistant") continue;
    if (replyExcerpt(item.text) === target.excerpt) return { ...target, entryId: item.entryId };
  }
  return target;
}

/** An id the marker can carry without breaking out of its comment. */
function markerId(entryId: string | undefined): string | undefined {
  return entryId && /^[A-Za-z0-9_-]+$/.test(entryId) ? entryId : undefined;
}

/**
 * The prompt the agent receives: the marker, the excerpt as `> ` lines, one
 * blank line, then the user's own words. The agent CLIs have no "reply to
 * message" of their own, so this text is the whole contract.
 */
export function formatReplyPrompt(target: ReplyTarget, text: string): string {
  const id = markerId(target.entryId);
  const marker = `<!-- bridge-reply${id ? ` ${id}` : ""}: ${FRAMING} -->`;
  const quote = quoteSelection(target.excerpt);
  const words = text.trim();
  return words ? `${marker}\n${quote}\n\n${words}` : `${marker}\n${quote}`;
}

/** A sent reply read back: what it replied to, and the words the user wrote. */
export interface ParsedReply {
  target: ReplyTarget;
  text: string;
}

/**
 * Read a stored prompt back into its reply and the user's words, so the
 * bubble draws a card instead of the raw quote. Anything that does not start
 * with the marker is an ordinary message and comes back as `null`.
 */
export function parseReplyPrompt(prompt: string): ParsedReply | null {
  if (!prompt.startsWith("<!-- bridge-reply")) return null;
  const lines = prompt.split("\n");
  const marker = MARKER.exec(lines[0] ?? "");
  if (!marker) return null;
  let index = 1;
  const quoted: string[] = [];
  while (index < lines.length && lines[index].startsWith(">")) {
    quoted.push(lines[index].replace(/^> ?/, ""));
    index += 1;
  }
  if (!quoted.length) return null;
  const text = lines.slice(index).join("\n").replace(/^\n/, "");
  return {
    target: { ...(marker[1] ? { entryId: marker[1] } : {}), excerpt: quoted.join("\n") },
    text,
  };
}

/**
 * A sent message as the user would copy it: the quote and their words, without
 * the marker line that is only there for the agent and the transcript.
 */
export function replyCopyText(prompt: string): string {
  const reply = parseReplyPrompt(prompt);
  if (!reply) return prompt;
  const quote = quoteSelection(reply.target.excerpt);
  return reply.text ? `${quote}\n\n${reply.text}` : quote;
}

/**
 * The excerpt as one line of plain words, for the banner and the card: the
 * Markdown a reply is written in (headings, emphasis, list marks, links) would
 * read as noise in a one-line preview.
 */
export function replyPreview(excerpt: string): string {
  return excerpt
    .split("\n")
    .map(line => line
      .replace(/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/, "")
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/(\*\*|__|\*|_|`)(.+?)\1/g, "$2")
      .trim())
    .filter(Boolean)
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
}
