// A link to a Bridge Notes page: `bridge://p/<id>`, the form the editor writes
// for a child page and the notes tools hand agents. A click opens the page
// inside Bridge; nothing about it ever reaches a browser.

// The id is the store's own (a UUID from the core, `note-…` in the mock); the
// core decides whether such a page exists.
const NOTE_LINK = /^bridge:\/\/p\/([A-Za-z0-9_-]{1,64})$/i;

/** The page id a note link names, or undefined for anything else. */
export function parseNoteLink(url: string): string | undefined {
  return NOTE_LINK.exec(url.trim())?.[1];
}

export const noteLink = (id: string) => `bridge://p/${id}`;
