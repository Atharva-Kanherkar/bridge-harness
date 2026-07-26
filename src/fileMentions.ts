const SAFE_FILE_MENTION = /^[A-Za-z0-9._/-]+$/;
const ACTIVE_MENTION = /(^|\s)@(?:("(?:\\.|[^"\\])*)|([^\s"]*))$/;

export function fileMentionQuery(text: string): string | undefined {
  const match = ACTIVE_MENTION.exec(text);
  if (!match) return undefined;
  if (match[2] == null) return match[3];
  const quotedBody = match[2].slice(1);
  try {
    return JSON.parse(`"${quotedBody}"`) as string;
  } catch {
    return quotedBody;
  }
}

export function formatFileMention(path: string): string {
  return SAFE_FILE_MENTION.test(path) ? `@${path}` : `@${JSON.stringify(path)}`;
}

export function applyFileMention(text: string, path: string): string {
  return text.replace(ACTIVE_MENTION, (_match, lead: string) => `${lead}${formatFileMention(path)} `);
}
