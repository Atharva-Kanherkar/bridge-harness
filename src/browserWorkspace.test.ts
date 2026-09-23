import { describe, expect, it } from "vitest";
import {
  BROWSER_STORAGE_PREFIX, MAX_BROWSER_HISTORY, MAX_BROWSER_TABS,
  browserWorkspaceReducer, defaultBrowserWorkspace, normalizeBrowserUrl,
  readBrowserWorkspace, writeBrowserWorkspace, type BrowserWorkspaceState,
} from "./browserWorkspace";

function storage() {
  const values = new Map<string, string>();
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

function initial(): BrowserWorkspaceState {
  return { version: 1, tabs: [{ id: "first", url: "", title: "", history: [], historyIndex: -1 }], activeTabId: "first", recentlyClosed: [] };
}

describe("browser address normalization", () => {
  it.each([
    ["[::1]", "http://[::1]/"],
    ["[::1]:8766", "http://[::1]:8766/"],
    ["[::1]:8766/path?q=hello#section", "http://[::1]:8766/path?q=hello#section"],
    ["localhost:1420?view=preview", "http://localhost:1420/?view=preview"],
    ["127.0.0.2:3000", "http://127.0.0.2:3000/"],
    ["app.localhost:3000", "http://app.localhost:3000/"],
    ["example.com/docs", "https://example.com/docs"],
    ["https://[::1]:8766", "https://[::1]:8766/"],
    ["  component design  ", "https://www.google.com/search?q=component%20design"],
  ])("normalizes %s", (value, expected) => {
    expect(normalizeBrowserUrl(value)).toBe(expected);
  });

  it.each(["", "  ", "javascript:alert(1)", "data:text/html,hello", "file:///tmp/example", "https://", "[::1]:99999", "[::::1]", "localhost:99999", "a".repeat(2049)])("rejects invalid or unsupported address %s", value => {
    expect(normalizeBrowserUrl(value)).toBeUndefined();
  });
});

describe("browser task tabs", () => {
  it("isolates navigation and preserves sibling tab metadata when switching", () => {
    let state = browserWorkspaceReducer(initial(), { type: "navigate", tabId: "first", url: "localhost:3000/one" });
    state = browserWorkspaceReducer(state, { type: "create", id: "second", url: "example.com" });
    state = browserWorkspaceReducer(state, { type: "navigate", tabId: "second", url: "example.org" });
    state = browserWorkspaceReducer(state, { type: "select", tabId: "first" });
    expect(state.activeTabId).toBe("first");
    expect(state.tabs[0].history).toEqual(["http://localhost:3000/one"]);
    expect(state.tabs[1].history).toEqual(["https://example.com/", "https://example.org/"]);
  });

  it("deduplicates native navigation confirmation and branches history after Back", () => {
    let state = initial();
    for (const url of ["example.com/one", "example.com/two", "example.com/three"]) state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url });
    state = browserWorkspaceReducer(state, { type: "committed-navigation", tabId: "first", url: "https://example.com/three", title: "Three" });
    expect(state.tabs[0].history).toHaveLength(3);
    expect(state.tabs[0].title).toBe("Three");
    state = browserWorkspaceReducer(state, { type: "back", tabId: "first" });
    state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url: "example.com/four" });
    expect(state.tabs[0].history).toEqual(["https://example.com/one", "https://example.com/two", "https://example.com/four"]);
    expect(browserWorkspaceReducer(state, { type: "forward", tabId: "first" })).toBe(state);
  });

  it("updates a redirect or replacement without introducing a false history entry", () => {
    let state = browserWorkspaceReducer(initial(), { type: "navigate", tabId: "first", url: "example.com/old" });
    state = browserWorkspaceReducer(state, { type: "committed-navigation", tabId: "first", url: "https://example.com/new", replace: true });
    expect(state.tabs[0].history).toEqual(["https://example.com/new"]);
    state = browserWorkspaceReducer(state, { type: "committed-navigation", tabId: "first", url: "https://example.com/link" });
    expect(state.tabs[0].history).toEqual(["https://example.com/new", "https://example.com/link"]);
  });
  it("traverses multiple history entries without discarding the forward branch", () => {
    let state = initial();
    for (const path of ["one", "two", "three", "four"]) state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url: `example.com/${path}` });
    const history = state.tabs[0].history;
    state = browserWorkspaceReducer(state, { type: "traverse", tabId: "first", url: "https://example.com/two" });
    expect(state.tabs[0].historyIndex).toBe(1);
    expect(state.tabs[0].history).toEqual(history);
    state = browserWorkspaceReducer(state, { type: "traverse", tabId: "first", url: "https://example.com/four" });
    expect(state.tabs[0].historyIndex).toBe(3);
    expect(state.tabs[0].history).toEqual(history);
  });
  it("resolves repeated URL traversals to the nearest stored entry", () => {
    let state = initial();
    for (const path of ["repeat", "two", "repeat", "four"]) state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url: `example.com/${path}` });
    state = browserWorkspaceReducer(state, { type: "traverse", tabId: "first", url: "https://example.com/repeat" });
    expect(state.tabs[0].historyIndex).toBe(2);
    expect(state.tabs[0].history).toHaveLength(4);
  });

  it("ignores events for a closed tab and invalid page-event URLs", () => {
    let state = browserWorkspaceReducer(initial(), { type: "create", id: "second" });
    state = browserWorkspaceReducer(state, { type: "close", tabId: "first" });
    expect(browserWorkspaceReducer(state, { type: "committed-navigation", tabId: "first", url: "https://example.com" })).toBe(state);
    expect(browserWorkspaceReducer(state, { type: "committed-navigation", tabId: "second", url: "search phrase" })).toBe(state);
    expect(browserWorkspaceReducer(state, { type: "select", tabId: "missing" })).toBe(state);
  });

  it("closes background tabs without switching, picks a neighbour and reopens with history", () => {
    let state = browserWorkspaceReducer(initial(), { type: "create", id: "second", url: "example.com" });
    state = browserWorkspaceReducer(state, { type: "create", id: "third", url: "example.org" });
    state = browserWorkspaceReducer(state, { type: "close", tabId: "first" });
    expect(state.activeTabId).toBe("third");
    state = browserWorkspaceReducer(state, { type: "close", tabId: "third" });
    expect(state.activeTabId).toBe("second");
    state = browserWorkspaceReducer(state, { type: "reopen" });
    expect(state.activeTabId).toBe("third");
    expect(state.tabs[1].history).toEqual(["https://example.org/"]);
    state = browserWorkspaceReducer(state, { type: "close", tabId: "second" });
    state = browserWorkspaceReducer(state, { type: "close", tabId: "third" });
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0].url).toBe("");
    expect(state.activeTabId).toBe(state.tabs[0].id);
  });

  it("bounds live histories and tabs without destroying existing tabs", () => {
    let state = initial();
    for (let n = 0; n < MAX_BROWSER_HISTORY + 5; n++) state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url: `example.com/${n}` });
    expect(state.tabs[0].history).toHaveLength(MAX_BROWSER_HISTORY);
    expect(state.tabs[0].history[0]).toBe("https://example.com/5");
    expect(state.tabs[0].historyIndex).toBe(MAX_BROWSER_HISTORY - 1);
    for (let n = 1; n < MAX_BROWSER_TABS; n++) state = browserWorkspaceReducer(state, { type: "create", id: `tab-${n}` });
    expect(browserWorkspaceReducer(state, { type: "create", id: "overflow" })).toBe(state);
    expect(browserWorkspaceReducer(state, { type: "create", id: "first" })).toBe(state);
  });
});

describe("browser metadata persistence", () => {
  it("restores independent task tabs, active selection and both history directions", () => {
    const memory = storage();
    let state = browserWorkspaceReducer(initial(), { type: "navigate", tabId: "first", url: "localhost:3000/one" });
    state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url: "localhost:3000/two" });
    state = browserWorkspaceReducer(state, { type: "back", tabId: "first" });
    state = browserWorkspaceReducer(state, { type: "create", id: "second", url: "example.com" });
    writeBrowserWorkspace("task-one", state, memory);
    writeBrowserWorkspace("task-two", initial(), memory);
    expect(readBrowserWorkspace("task-one", memory)).toEqual(state);
    expect(readBrowserWorkspace("task-two", memory)).toEqual(initial());
  });

  it("persists only allowed metadata and drops sensitive pages including titles", () => {
    const memory = storage();
    let state = initial();
    for (const [id, url] of [
      ["credentials", "https://user:password@example.com/"],
      ["token", "https://example.com/?access_token=secret"],
      ["fragment", "https://example.com/#id_token=secret"],
      ["code", "https://example.com/callback?code=secret"],
    ]) state = browserWorkspaceReducer(state, { type: "create", id, url });
    state = browserWorkspaceReducer(state, { type: "update-title", tabId: "token", title: "secret page" });
    const contaminated = { ...state, formData: "never-store-this", screenshot: "never-store-image" };
    writeBrowserWorkspace("task", contaminated, memory);
    const raw = memory.values.get(BROWSER_STORAGE_PREFIX + "task")!;
    expect(raw).not.toMatch(/secret|password|never-store|formData|screenshot/);
    expect(readBrowserWorkspace("task", memory).tabs.every(tab => tab.url === "" && tab.history.length === 0)).toBe(true);
  });

  it("filters sensitive history while keeping the current index and safe URLs", () => {
    const memory = storage();
    let state = initial();
    for (const url of ["example.com/one", "https://example.com/auth?code=private", "example.com/two?view=preview#details", "example.com/three"]) state = browserWorkspaceReducer(state, { type: "navigate", tabId: "first", url });
    state = browserWorkspaceReducer(state, { type: "back", tabId: "first" });
    writeBrowserWorkspace("task", state, memory);
    const tab = readBrowserWorkspace("task", memory).tabs[0];
    expect(tab.history).toEqual(["https://example.com/one", "https://example.com/two?view=preview#details", "https://example.com/three"]);
    expect(tab.historyIndex).toBe(1);
    expect(tab.url).toBe(tab.history[1]);
  });

  it("repairs duplicate IDs, missing active tab and invalid history indices", () => {
    const memory = storage();
    memory.setItem(BROWSER_STORAGE_PREFIX + "task", JSON.stringify({ version: 1, activeTabId: "missing", tabs: [
      { id: "good", url: "https://example.com/", title: 5, history: ["javascript:alert(1)"], historyIndex: 999 },
      { id: "good", url: "https://example.org/" },
      { id: {}, url: "https://example.net/" },
    ], recentlyClosed: [] }));
    const state = readBrowserWorkspace("task", memory);
    expect(state.tabs).toHaveLength(1);
    expect(state.activeTabId).toBe("good");
    expect(state.tabs[0]).toEqual({ id: "good", url: "https://example.com/", title: "", history: ["https://example.com/"], historyIndex: 0 });
  });

  it.each(["not json", "null", '{"version":2,"tabs":[]}', '{"version":1,"tabs":[]}', " ".repeat(4_000_001)])("falls back safely for malformed storage %#", raw => {
    const memory = storage();
    memory.setItem(BROWSER_STORAGE_PREFIX + "task", raw);
    const state = readBrowserWorkspace("task", memory);
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0].url).toBe("");
  });

  it("keeps browsing usable when storage is denied or full", () => {
    const denied = { getItem: () => { throw new Error("denied"); }, setItem: () => { throw new Error("quota"); } };
    expect(readBrowserWorkspace("task", denied).tabs).toHaveLength(1);
    expect(() => writeBrowserWorkspace("task", defaultBrowserWorkspace("example.com"), denied)).not.toThrow();
  });
});
