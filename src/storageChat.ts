// The Storage page's standing chat, remembered by id so every visit reopens
// the same conversation. A stale id (the chat was archived or deleted) just
// means the next question starts a new one.

export const STORAGE_CHAT_KEY = "bridge.storage.chatId";
export const STORAGE_CHAT_TITLE = "Storage";

export function readStorageChatId(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(STORAGE_CHAT_KEY);
}

export function writeStorageChatId(id: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(STORAGE_CHAT_KEY, id);
}
