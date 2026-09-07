// `$harness` composer shortcut: typing `$codex do you think we're right?`
// anywhere skips the current session and starts a fresh direct chat pinned
// to that harness instead. Distinct from `/name` slash commands, which stay
// inside the current direct chat and only switch its harness.
const HARNESS_SHORTCUT = /^\$([A-Za-z0-9._-]+)\s+([\s\S]*)$/;

/** A completed `$id <message>` shortcut, or null if the composer doesn't have one yet. */
export function parseHarnessShortcut(text: string): { harnessId: string; rest: string } | null {
  const match = HARNESS_SHORTCUT.exec(text);
  if (!match) return null;
  const rest = match[2].trim();
  if (!rest) return null;
  return { harnessId: match[1], rest };
}

/** The `$id` token being typed at the very end of the composer with nothing after it yet — drives the autocomplete dropdown. */
export function harnessShortcutQuery(text: string): string | undefined {
  return /^\$([^\s]*)$/.exec(text)?.[1];
}
