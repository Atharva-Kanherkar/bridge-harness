// The Storage page's standing chat, remembered by id so every visit reopens
// the same conversation. A stale id (the chat was archived or deleted) just
// means the next question starts a new one.
//
// The key is versioned: chats from before the storage agent had a system
// prompt of its own carried their brief in the first message, so they are left
// in the sidebar and a fresh purposed chat takes the rail.

export const STORAGE_CHAT_KEY = "bridge.storage.chatId.v2";
export const STORAGE_CHAT_TITLE = "Storage";
/** Selects the storage agent's system prompt on the backend. */
export const STORAGE_CHAT_PURPOSE = "storage";

export function readStorageChatId(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(STORAGE_CHAT_KEY);
}

export function writeStorageChatId(id: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(STORAGE_CHAT_KEY, id);
}
