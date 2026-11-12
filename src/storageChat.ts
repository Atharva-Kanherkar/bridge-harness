// The Storage page's chats, remembered by id. The rail opens blank on each
// visit and lists the remembered ones under Recent; a stale id (the chat was
// archived or deleted) simply drops out of that list.
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

// Every storage chat the rail has started, newest first, so a fresh visit can
// open on a blank rail and still reach earlier conversations.
export const STORAGE_CHAT_HISTORY_KEY = "bridge.storage.chatHistory.v1";
const HISTORY_LIMIT = 20;

export function readStorageChatHistory(): string[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_CHAT_HISTORY_KEY) ?? "[]");
    const ids = Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [];
    // Chats started before the history existed are still the newest one.
    const current = readStorageChatId();
    return current && !ids.includes(current) ? [current, ...ids] : ids;
  } catch { return []; }
}

export function rememberStorageChat(id: string): string[] {
  const next = [id, ...readStorageChatHistory().filter(item => item !== id)].slice(0, HISTORY_LIMIT);
  writeStorageChatId(id);
  if (typeof localStorage !== "undefined") localStorage.setItem(STORAGE_CHAT_HISTORY_KEY, JSON.stringify(next));
  return next;
}
