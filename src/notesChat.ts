// The Notes page's standing chat, remembered by id so every visit reopens the
// same conversation. A stale id (the chat was archived or deleted) just means
// the next question starts a new one.

export const NOTES_CHAT_KEY = "bridge.notes.chatId.v1";
export const NOTES_CHAT_TITLE = "Notes";
/** Selects the notes agent's system prompt on the backend. */
export const NOTES_CHAT_PURPOSE = "notes";

export function readNotesChatId(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(NOTES_CHAT_KEY);
}

export function writeNotesChatId(id: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(NOTES_CHAT_KEY, id);
}
