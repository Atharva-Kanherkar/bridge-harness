// A link to a Bridge Notes page: `bridge://p/<id>`, the form the editor writes
// for a child page and the notes tools hand agents. A click opens the page
// inside Bridge; nothing about it ever reaches a browser.

const NOTE_LINK = /^bridge:\/\/p\/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i;

/** The page id a note link names, or undefined for anything else. */
export function parseNoteLink(url: string): string | undefined {
  return NOTE_LINK.exec(url.trim())?.[1].toLowerCase();
}

export const noteLink = (id: string) => `bridge://p/${id}`;
