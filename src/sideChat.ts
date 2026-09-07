// A side chat (`/btw`, `/side`, or a text selection): a consult the user opens
// beside the chat they are in. The side chat reads the parent conversation's
// projected context but never appends to it and never resolves the parent's
// approvals — its replies live in its own session, in the sidebar afterwards.
// Distinct from the `$harness` composer shortcut (a delegation to a named
// harness) and from provider TUI commands: Bridge owns `/btw` and `/side` at
// the composer level, backed by a Codex native thread fork when available.

const SIDE_CHAT_COMMAND = /^\/(btw|side)\b\s*([\s\S]*)$/i;

/** The Bridge-owned side-chat command names, for pickers that must not offer
 *  them where they cannot work (an aside is pinned and cannot nest one). */
export const SIDE_CHAT_COMMANDS: readonly string[] = ["btw", "side"];

/** A completed `/btw <question>` or `/side <question>` turn, or null when the
 *  composer text is not a side-chat command. An empty question is still a
 *  side-chat request — callers decide what an empty one means. */
export function parseSideChatCommand(text: string): { command: "btw" | "side"; query: string } | null {
  const match = SIDE_CHAT_COMMAND.exec(text.trim());
  if (!match) return null;
  return { command: match[1].toLowerCase() as "btw" | "side", query: match[2].trim() };
}

/** Quote a transcript selection as the side chat's first message. Every
 *  selected line becomes one blockquote line, so the aside's transcript shows
 *  the excerpt as the excerpt it is rather than prose the user never wrote.
 *  Blank lines are dropped: a trailing newline (the common selection tail)
 *  must not leave an empty quote line, and interior blanks collapse into one
 *  continuous blockquote either way. */
export function quoteSelection(text: string): string {
  return text
    .split("\n")
    .map(line => line.trimEnd())
    .filter(line => line.length > 0)
    .map(line => `> ${line}`)
    .join("\n");
}
