import { invoke, isTauri } from "@tauri-apps/api/core";

export type BrowserPageSnapshot = {
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  backUrl?: string | null;
  forwardUrl?: string | null;
  loading: boolean;
  navigationId: number;
  error?: string | null;
  selection?: { selector: string; snippet: string; bounds: { x: number; y: number; width: number; height: number } } | null;
  popupUrl?: string | null;
  cancelled?: boolean;
  historyAction?: "push" | "replace" | "traverse" | null;
  shortcut?: "address" | "new_tab" | "close_tab" | "reopen_tab" | null;
};
export type BrowserAction = "back" | "forward" | "reload" | "stop" | "inspect" | "cancel_inspect";
export const hasNativeBrowser = () => isTauri();
export function browserCommand<T = void>(command: string, sessionId: string, tabId: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>(`plugin:embedded-browser|${command}`, { sessionId, tabId, ...args });
}

/** Native children are above the DOM. Never leave one over app overlays. */
export function browserViewportVisible(element: HTMLElement, visible: boolean): boolean {
  if (!visible || document.visibilityState === "hidden" || !element.getClientRects().length) return false;
  return !Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"], [role="alertdialog"], [aria-modal="true"], [role="menu"], [role="listbox"], [data-browser-occluder]'))
    .some(dialog => dialog.getClientRects().length > 0 && getComputedStyle(dialog).visibility !== "hidden");
}

const pageValidators = new Map<string, () => Promise<BrowserPageSnapshot | undefined>>();
const pageKey = (sessionId: string, tabId: string) => JSON.stringify([sessionId, tabId]);
export function registerBrowserPageValidator(sessionId: string, tabId: string, read: () => Promise<BrowserPageSnapshot | undefined>): () => void {
  const key = pageKey(sessionId, tabId);
  pageValidators.set(key, read);
  return () => { if (pageValidators.get(key) === read) pageValidators.delete(key); };
}
export async function validateBrowserSelectionPage(sessionId: string, tabId: string, navigationId: number): Promise<boolean> {
  const read = pageValidators.get(pageKey(sessionId, tabId));
  if (!read) return false;
  try {
    const snapshot = await read();
    return pageValidators.get(pageKey(sessionId, tabId)) === read && !!snapshot && !snapshot.loading && !snapshot.error && snapshot.navigationId === navigationId;
  } catch { return false; }
}
