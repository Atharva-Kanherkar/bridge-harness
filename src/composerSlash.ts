import type { SlashCommand } from "./types";

export type ComposerSlashToken = { start: number; end: number; query: string; leading: boolean };

/** Only an explicit, whitespace-delimited slash at the caret opens discovery. */
export function composerSlashToken(text: string, start = text.length, end = start): ComposerSlashToken | undefined {
  if (start !== end || start < 0 || start > text.length) return;
  const prefix = text.slice(0, start);
  // Slash-shaped text inside quoted code is literal, as are URLs and paths.
  if ((prefix.match(/`/g)?.length ?? 0) % 2) return;
  const match = /(?:^|\s)\/([\w:-]*)$/.exec(prefix);
  if (!match) return;
  const tokenStart = start - match[1].length - 1;
  const suffix = /^[\w:-]*/.exec(text.slice(start))![0];
  const tokenEnd = start + suffix.length;
  if (tokenEnd < text.length && !/\s/.test(text[tokenEnd])) return;
  return { start: tokenStart, end: tokenEnd, query: match[1], leading: !text.slice(0, tokenStart).trim() };
}

export function composerSlashMatches(commands: SlashCommand[], token: ComposerSlashToken | undefined): SlashCommand[] {
  if (!token) return [];
  const query = token.query.toLowerCase();
  return commands
    .filter(command => token.leading || ["skill", "command", "prompt"].includes(command.kind))
    .filter(command => !query || command.name.toLowerCase().includes(query) || command.description.toLowerCase().includes(query))
    .sort((a, b) => Number(b.name.toLowerCase().startsWith(query)) - Number(a.name.toLowerCase().startsWith(query))
      || a.name.localeCompare(b.name));
}

export function insertComposerSlash(text: string, token: ComposerSlashToken, name: string): { text: string; caret: number } {
  const inserted = `/${name}`;
  const suffix = text.slice(token.end);
  const separator = suffix.startsWith(" ") ? "" : " ";
  const next = text.slice(0, token.start) + inserted + separator + suffix;
  return { text: next, caret: token.start + inserted.length + 1 };
}
