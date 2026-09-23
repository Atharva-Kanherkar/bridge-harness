/** Browser page metadata belongs to a task, independently of the dock layout.
 * Runtime views, form contents and inspected page content never enter this record. */
export type BrowserTabState = {
  id: string;
  url: string;
  title: string;
  history: string[];
  historyIndex: number;
};

export type BrowserWorkspaceState = {
  version: 1;
  tabs: BrowserTabState[];
  activeTabId: string;
  recentlyClosed: BrowserTabState[];
};

export type BrowserWorkspaceAction =
  | { type: "create"; id?: string; url?: string }
  | { type: "select"; tabId: string }
  | { type: "close"; tabId: string }
  | { type: "reopen" }
  | { type: "navigate"; tabId: string; url: string }
  | { type: "committed-navigation"; tabId: string; url: string; replace?: boolean; title?: string }
  | { type: "back"; tabId: string }
  | { type: "forward"; tabId: string }
  | { type: "traverse"; tabId: string; url: string }
  | { type: "update-title"; tabId: string; title: string };

export const MAX_BROWSER_TABS = 20;
export const MAX_BROWSER_CLOSED_TABS = 10;
export const MAX_BROWSER_HISTORY = 50;
export const MAX_BROWSER_URL_LENGTH = 2048;
export const MAX_BROWSER_TITLE_LENGTH = 160;
export const BROWSER_STORAGE_PREFIX = "bridge.browser.v1.";
const MAX_STORAGE_LENGTH = 4_000_000;

function httpUrl(value: string): URL | undefined {
  if (value.length > MAX_BROWSER_URL_LENGTH) return undefined;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:" ? url : undefined;
  } catch { return undefined; }
}

/** Resolve an omnibox input without ever permitting executable URL schemes. */
export function normalizeBrowserUrl(input: string): string | undefined {
  const value = input.trim();
  if (!value || value.length > MAX_BROWSER_URL_LENGTH) return undefined;
  if (/^https?:\/\//i.test(value)) return httpUrl(value)?.href;

  // Ports are not schemes. Admit bracketed IPv6 and host:port before the
  // explicit-scheme guard; URL validates IPv6 syntax and port range for us.
  const hostShaped = /^(?:\[[\da-f:.]+\]|localhost|(?:[\w-]+\.)+[\w-]+)(?::\d+)?(?:[/?#][^\s]*)?$/i.test(value);
  if (hostShaped) {
    const parsed = httpUrl(`https://${value}`);
    if (!parsed) return undefined;
    if (parsed.hostname === "localhost" || parsed.hostname.endsWith(".localhost") || parsed.hostname === "[::1]" || /^127(?:\.\d{1,3}){3}$/.test(parsed.hostname)) parsed.protocol = "http:";
    return parsed.href;
  }
  if (/^[a-z][a-z\d+.-]*:/i.test(value) || value.startsWith("[")) return undefined;
  return `https://www.google.com/search?q=${encodeURIComponent(value)}`;
}

function newTabId(): string { return crypto.randomUUID(); }
function blankTab(id = newTabId()): BrowserTabState {
  return { id, url: "", title: "", history: [], historyIndex: -1 };
}

export function defaultBrowserWorkspace(initialUrl = ""): BrowserWorkspaceState {
  const tab = blankTab();
  const url = normalizeBrowserUrl(initialUrl);
  if (url) Object.assign(tab, { url, history: [url], historyIndex: 0 });
  return { version: 1, tabs: [tab], activeTabId: tab.id, recentlyClosed: [] };
}

function navigateTab(tab: BrowserTabState, url: string, replace = false, title?: string): BrowserTabState {
  const nextTitle = title === undefined ? (url === tab.url ? tab.title : "") : title.slice(0, MAX_BROWSER_TITLE_LENGTH);
  if (url === tab.url) return nextTitle === tab.title ? tab : { ...tab, title: nextTitle };
  let history = tab.history.slice(0, tab.historyIndex + 1);
  if (replace && history.length) history[history.length - 1] = url;
  else history.push(url);
  history = history.slice(-MAX_BROWSER_HISTORY);
  return { ...tab, url, title: nextTitle, history, historyIndex: history.length - 1 };
}

export function browserWorkspaceReducer(state: BrowserWorkspaceState, action: BrowserWorkspaceAction): BrowserWorkspaceState {
  switch (action.type) {
    case "create": {
      if (state.tabs.length >= MAX_BROWSER_TABS || (action.id && state.tabs.some(tab => tab.id === action.id))) return state;
      let tab = blankTab(action.id);
      const url = normalizeBrowserUrl(action.url ?? "");
      if (url) tab = navigateTab(tab, url);
      return { ...state, tabs: [...state.tabs, tab], activeTabId: tab.id };
    }
    case "select":
      return state.tabs.some(tab => tab.id === action.tabId) ? { ...state, activeTabId: action.tabId } : state;
    case "close": {
      const index = state.tabs.findIndex(tab => tab.id === action.tabId);
      if (index < 0) return state;
      const closed = state.tabs[index];
      const tabs = state.tabs.filter(tab => tab.id !== action.tabId);
      if (!tabs.length) tabs.push(blankTab());
      return {
        ...state, tabs,
        activeTabId: state.activeTabId === action.tabId ? tabs[Math.min(index, tabs.length - 1)].id : state.activeTabId,
        recentlyClosed: [...state.recentlyClosed.filter(tab => tab.id !== closed.id), closed].slice(-MAX_BROWSER_CLOSED_TABS),
      };
    }
    case "reopen": {
      if (state.tabs.length >= MAX_BROWSER_TABS || !state.recentlyClosed.length) return state;
      const previous = state.recentlyClosed[state.recentlyClosed.length - 1];
      const tab = state.tabs.some(candidate => candidate.id === previous.id) ? { ...previous, id: newTabId() } : previous;
      return { ...state, tabs: [...state.tabs, tab], activeTabId: tab.id, recentlyClosed: state.recentlyClosed.slice(0, -1) };
    }
    default: {
      let changed = false;
      const tabs = state.tabs.map(tab => {
        if (tab.id !== action.tabId) return tab;
        let next = tab;
        if (action.type === "back" || action.type === "forward") {
          const historyIndex = tab.historyIndex + (action.type === "back" ? -1 : 1);
          if (historyIndex >= 0 && historyIndex < tab.history.length) next = { ...tab, historyIndex, url: tab.history[historyIndex], title: "" };
        } else if (action.type === "traverse") {
          const url = httpUrl(action.url)?.href;
          if (url) {
            let historyIndex = -1;
            tab.history.forEach((entry, index) => {
              if (entry === url && (historyIndex < 0 || Math.abs(index - tab.historyIndex) < Math.abs(historyIndex - tab.historyIndex))) historyIndex = index;
            });
            next = historyIndex < 0 ? navigateTab(tab, url) : { ...tab, url, historyIndex, title: url === tab.url ? tab.title : "" };
          }
        } else if (action.type === "update-title") {
          next = { ...tab, title: action.title.slice(0, MAX_BROWSER_TITLE_LENGTH) };
        } else {
          // Page events must contain URLs, never search-shaped strings.
          const url = action.type === "navigate" ? normalizeBrowserUrl(action.url) : httpUrl(action.url)?.href;
          if (url) next = navigateTab(tab, url, action.type === "committed-navigation" && action.replace, action.type === "committed-navigation" ? action.title : undefined);
        }
        changed ||= next !== tab;
        return next;
      });
      return changed ? { ...state, tabs } : state;
    }
  }
}

/** Credential-bearing or authorization URLs are omitted, not rewritten into a
 * different page. Ordinary query strings and anchors remain useful on restore. */
function persistableUrl(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const url = httpUrl(value);
  if (!url || url.username || url.password) return undefined;
  const sensitiveKey = /(?:token|secret|password|passwd|credential|signature|authorization|api.?key|access.?key|session.?id)|^(?:auth|code|key|ticket|sid|samlresponse|assertion)$/i;
  let sensitive = false;
  url.searchParams.forEach((_value, key) => { sensitive ||= sensitiveKey.test(key); });
  const fragment = new URLSearchParams(url.hash.slice(1));
  fragment.forEach((_value, key) => { sensitive ||= sensitiveKey.test(key); });
  if (sensitive) return undefined;
  return url.href;
}

function restoreTab(value: unknown): BrowserTabState | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== "string" || !/^[\w-]{1,100}$/.test(record.id) || typeof record.url !== "string") return undefined;
  const url = persistableUrl(record.url);
  if (!url) return blankTab(record.id);
  const rawHistory = Array.isArray(record.history) ? record.history : [];
  const rawIndex = typeof record.historyIndex === "number" && Number.isInteger(record.historyIndex) ? record.historyIndex : -1;
  // Preserve a valid current entry and both directions. Malformed histories
  // fall back to the known current page, never an unrelated indexed URL.
  let history = [url];
  let historyIndex = 0;
  if (rawIndex >= 0 && rawIndex < rawHistory.length && persistableUrl(rawHistory[rawIndex]) === url) {
    const start = Math.max(0, rawIndex - MAX_BROWSER_HISTORY + 1);
    const entries = rawHistory.slice(start, start + MAX_BROWSER_HISTORY).map(persistableUrl);
    historyIndex = entries.slice(0, rawIndex - start).filter(Boolean).length;
    history = entries.filter((entry): entry is string => !!entry);
  }
  return { id: record.id, url, title: typeof record.title === "string" ? record.title.slice(0, MAX_BROWSER_TITLE_LENGTH) : "", history, historyIndex };
}

function restoreWorkspace(value: unknown): BrowserWorkspaceState | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.tabs)) return undefined;
  const ids = new Set<string>();
  const parseTabs = (values: unknown[], max: number) => values.slice(0, max).flatMap(value => {
    const tab = restoreTab(value);
    if (!tab || ids.has(tab.id)) return [];
    ids.add(tab.id);
    return [tab];
  });
  const tabs = parseTabs(record.tabs, MAX_BROWSER_TABS);
  if (!tabs.length) return undefined;
  const recentlyClosed = parseTabs(Array.isArray(record.recentlyClosed) ? record.recentlyClosed.slice(-MAX_BROWSER_CLOSED_TABS) : [], MAX_BROWSER_CLOSED_TABS);
  const activeTabId = tabs.find(tab => tab.id === record.activeTabId)?.id ?? tabs[0].id;
  return { version: 1, tabs, activeTabId, recentlyClosed };
}

export function readBrowserWorkspace(sessionId: string, storage?: Pick<Storage, "getItem">): BrowserWorkspaceState {
  try {
    const raw = (storage ?? localStorage).getItem(BROWSER_STORAGE_PREFIX + sessionId);
    if (raw && raw.length <= MAX_STORAGE_LENGTH) {
      const restored = restoreWorkspace(JSON.parse(raw));
      if (restored) return restored;
    }
  } catch { /* Corrupt or unavailable storage does not stop browsing. */ }
  return defaultBrowserWorkspace();
}

export function writeBrowserWorkspace(sessionId: string, state: BrowserWorkspaceState, storage?: Pick<Storage, "setItem">): void {
  try {
    const safe = restoreWorkspace(state);
    if (safe) (storage ?? localStorage).setItem(BROWSER_STORAGE_PREFIX + sessionId, JSON.stringify(safe));
  } catch { /* Quota or read-only storage does not stop browsing. */ }
}
