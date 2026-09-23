// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserPageHandle, BrowserPageProps } from "./BrowserPage";
import type { BrowserPageSnapshot } from "../browserRuntime";
import { readBrowserWorkspace } from "../browserWorkspace";
import { SimpleBrowser } from "./SimpleBrowser";

type MockPage = { props: BrowserPageProps; handle: BrowserPageHandle; current: BrowserPageSnapshot };
const harness = vi.hoisted(() => ({ pages: new Map<string, MockPage>(), mounts: new Map<string, number>(), openExternalUrl: vi.fn(async (_url: string) => {}) }));
vi.mock("../externalLinks", () => ({ openExternalUrl: harness.openExternalUrl }));
vi.mock("../browserRuntime", () => ({ hasNativeBrowser: () => true }));
vi.mock("./BrowserPage", async () => {
  const React = await import("react");
  return { BrowserPage: React.forwardRef<BrowserPageHandle, BrowserPageProps>(function MockBrowserPage(props, ref) {
    const stable = React.useRef<MockPage>();
    if (!stable.current) {
      const page: MockPage = {
        props,
        current: { url: props.initialUrl, title: "", canGoBack: false, canGoForward: false, loading: false, navigationId: 1 },
        handle: { navigate: vi.fn(async (_url: string) => {}), action: vi.fn(async () => {}), snapshot: vi.fn(async () => page.current) },
      };
      stable.current = page;
    }
    const page = stable.current;
    page.props = props;
    React.useImperativeHandle(ref, () => page.handle, [page]);
    React.useEffect(() => {
      harness.pages.set(props.tabId, page);
      harness.mounts.set(props.tabId, (harness.mounts.get(props.tabId) ?? 0) + 1);
      return () => { harness.pages.delete(props.tabId); };
    }, [page, props.tabId]);
    return <div data-mock-page={props.tabId} data-visible={props.visible ? "true" : "false"} />;
  }) };
});

let container: HTMLDivElement;
let root: Root;
const inputSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
const textareaSetter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
    clear: () => { values.clear(); },
    key: (index: number) => Array.from(values.keys())[index] ?? null,
    get length() { return values.size; },
  });
  harness.pages.clear(); harness.mounts.clear(); vi.clearAllMocks();
  container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root?.unmount()); container?.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(element => element.getAttribute("aria-label") === label || element.textContent === label);
  if (!found) throw new Error(`Missing button: ${label}`);
  return found;
}
function address(): HTMLInputElement { return container.querySelector<HTMLInputElement>('input[aria-label="Address"]')!; }
function activePage(): MockPage {
  const selected = container.querySelector('[role="tab"][aria-selected="true"]')!;
  const page = harness.pages.get(selected.id.replace("browser-tab-", ""));
  if (!page) throw new Error("Active page was not mounted");
  return page;
}
async function click(label: string) { await act(async () => button(label).click()); }
async function submit(value: string) {
  await act(async () => { inputSetter.call(address(), value); address().dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => { container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
}
async function emit(page: MockPage, update: Partial<BrowserPageSnapshot>) {
  page.current = { ...page.current, ...update };
  await act(async () => page.props.onSnapshot(page.current));
}
async function key(element: Element, value: string, extras: KeyboardEventInit = {}) {
  await act(async () => { element.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...extras })); });
}
const selection = { selector: "main > button:nth-of-type(1)", snippet: '<button onclick="steal()">Save<input value="private value"></button>', bounds: { x: 12, y: 30, width: 120, height: 32 } };

describe("SimpleBrowser tabs and navigation", () => {
  it("keeps independent page handles mounted across tab and dock visibility switches", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/one" />));
    const first = activePage();
    await click("New browser tab");
    const second = activePage();
    await submit("[::1]:8766/two");
    expect(second.handle.navigate).toHaveBeenCalledWith("http://[::1]:8766/two");
    expect(first.handle.navigate).not.toHaveBeenCalled();
    expect(first.props.visible).toBe(false);
    await act(async () => container.querySelector<HTMLButtonElement>('[role="tab"]')!.click());
    expect(activePage()).toBe(first);
    expect(address().value).toBe("http://localhost:3000/one");
    expect(harness.mounts.get(first.props.tabId)).toBe(1);
    expect(harness.mounts.get(second.props.tabId)).toBe(1);
    await act(async () => root.render(<SimpleBrowser sessionId="task" visible={false} />));
    expect(first.props.visible).toBe(false);
    await act(async () => root.render(<SimpleBrowser sessionId="task" visible />));
    expect(activePage()).toBe(first);
    expect(first.props.visible).toBe(true);
  });
  it("closes and reopens a page with its URL, leaving its sibling mounted", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="example.com/first" />));
    const first = activePage();
    await click("New browser tab"); await submit("example.org/second");
    const second = activePage();
    await key(address(), "w", { ctrlKey: true });
    expect(harness.pages.has(second.props.tabId)).toBe(false);
    expect(activePage()).toBe(first);
    await click("Reopen closed browser tab");
    expect(address().value).toBe("https://example.org/second");
    expect(activePage().props.tabId).toBe(second.props.tabId);
    expect(harness.mounts.get(first.props.tabId)).toBe(1);
    expect(harness.mounts.get(second.props.tabId)).toBe(2);
  });
  it("supports roving tab focus, close, reopen and focus-address keyboard controls", async () => {
    // Hold frames until a user has moved focus. Even an already-dequeued stale
    // callback must not steal focus back from the tab chosen with the keyboard.
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    await act(async () => root.render(<SimpleBrowser sessionId="task" />));
    await key(address(), "t", { ctrlKey: true }); await key(address(), "t", { ctrlKey: true });
    const tabs = () => Array.from(container.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
    expect(tabs()).toHaveLength(3);
    await key(tabs()[2], "Home");
    await act(async () => { frames.forEach(callback => callback(0)); });
    expect(tabs()[0].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tabs()[0]);
    await key(tabs()[0], "ArrowLeft");
    expect(document.activeElement).toBe(tabs()[2]);
    await key(tabs()[2], "Delete");
    expect(tabs()).toHaveLength(2);
    await key(address(), "t", { ctrlKey: true, shiftKey: true });
    expect(tabs()).toHaveLength(3);
    await key(tabs()[2], "l", { ctrlKey: true });
    expect(document.activeElement).toBe(address());
    expect(tabs().filter(tab => tab.tabIndex === 0)).toHaveLength(1);
  });
  it("uses the committed linked page for the address, reload and external open", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/one" />));
    const page = activePage();
    await emit(page, { url: "http://localhost:3000/two", title: "Second page", navigationId: 2, canGoBack: true, backUrl: "http://localhost:3000/one" });
    expect(address().value).toBe("http://localhost:3000/two");
    expect(container.querySelector('[role="tab"]')?.textContent).toContain("Second page");
    await click("Reload");
    expect(page.handle.action).toHaveBeenCalledWith("reload");
    expect(page.handle.navigate).not.toHaveBeenCalled();
    await click("Open in system browser");
    expect(harness.openExternalUrl).toHaveBeenCalledWith("http://localhost:3000/two");
    await click("Back"); expect(page.handle.action).toHaveBeenCalledWith("back");
  });
  it("navigates stored history when a restored page has no native history", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="example.com/one" />));
    await submit("example.com/two");
    const page = activePage();
    await click("Back");
    expect(page.handle.navigate).toHaveBeenLastCalledWith("https://example.com/one");
    expect(address().value).toBe("https://example.com/one");
    await click("Forward");
    expect(page.handle.navigate).toHaveBeenLastCalledWith("https://example.com/two");
  });
  it("ignores snapshots for a closed tab and older navigation", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="example.com/one" />));
    const first = activePage();
    await emit(first, { url: "https://example.com/new", navigationId: 4 });
    await emit(first, { url: "https://example.com/stale", navigationId: 3 });
    expect(address().value).toBe("https://example.com/new");
    await key(address(), "w", { ctrlKey: true });
    await emit(first, { url: "https://example.com/closed", navigationId: 5 });
    expect(address().value).toBe("");
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(1);
  });
  it("keeps a requested URL while an earlier native snapshot is still in flight", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="example.com/old" />));
    const page = activePage();
    await emit(page, { navigationId: 7 });
    await submit("example.com/requested");
    await emit(page, { url: "https://example.com/old", navigationId: 7 });
    expect(address().value).toBe("https://example.com/requested");
    await emit(page, { url: "https://example.com/requested", navigationId: 8 });
    expect(address().value).toBe("https://example.com/requested");
    expect(readBrowserWorkspace("task").tabs[0].history).toEqual(["https://example.com/old", "https://example.com/requested"]);
  });
  it("preserves forward entries when the page traverses several history entries", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="example.com/one" />));
    const page = activePage();
    await emit(page, { url: "https://example.com/two", navigationId: 2 });
    await emit(page, { url: "https://example.com/three", navigationId: 3 });
    await emit(page, { url: "https://example.com/one", navigationId: 4, historyAction: "traverse", canGoForward: true });
    expect(address().value).toBe("https://example.com/one");
    const tab = readBrowserWorkspace("task").tabs[0];
    expect(tab.history).toEqual(["https://example.com/one", "https://example.com/two", "https://example.com/three"]);
    expect(tab.historyIndex).toBe(0);
    expect(button("Forward").disabled).toBe(false);
  });
  it("rejects executable addresses without navigating", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" />));
    await submit("javascript:alert(1)");
    expect(activePage().handle.navigate).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("HTTP or HTTPS");
  });
});

describe("SimpleBrowser loading and restoration", () => {
  it("keeps slow pages mounted, offers stop and accepts their eventual completion", async () => {
    vi.useFakeTimers();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/slow" />));
    const page = activePage();
    await emit(page, { loading: true, navigationId: 2 });
    await act(async () => { vi.advanceTimersByTime(9000); });
    expect(container.textContent).toContain("Still loading");
    expect(container.textContent).not.toContain("can't be embedded");
    expect(container.textContent).not.toContain("Page could not load");
    expect(harness.pages.get(page.props.tabId)).toBe(page);
    expect(page.props.visible).toBe(true);
    await click("Stop loading"); expect(page.handle.action).toHaveBeenCalledWith("stop");
    await emit(page, { loading: false, title: "Slow page finished" });
    expect(container.textContent).not.toContain("Still loading");
    expect(button("Reload").disabled).toBe(false);
  });
  it("shows a genuine failure and lets retry recover in the same page", async () => {
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/offline" />));
    const page = activePage();
    await emit(page, { error: "The server is offline.", loading: false, navigationId: 2 });
    expect(container.textContent).toContain("Page could not load");
    expect(container.textContent).toContain("The server is offline.");
    await click("Retry"); expect(page.handle.action).toHaveBeenCalledWith("reload");
    await emit(page, { error: null, loading: false, navigationId: 3 });
    expect(container.textContent).not.toContain("Page could not load");
    expect(page.props.visible).toBe(true);
    expect(harness.mounts.get(page.props.tabId)).toBe(1);
  });
  it("restores each task's tabs and active page after unmount, without copying another task", async () => {
    await act(async () => root.render(<SimpleBrowser key="one" sessionId="one" initialUrl="example.com/one" />));
    await click("New browser tab"); await submit("example.org/two");
    expect(readBrowserWorkspace("one").tabs).toHaveLength(2);
    await act(async () => root.render(<SimpleBrowser key="two" sessionId="two" />));
    expect(address().value).toBe("");
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(1);
    await submit("localhost:3000/task-two");
    await act(async () => root.render(<SimpleBrowser key="one" sessionId="one" />));
    expect(address().value).toBe("https://example.org/two");
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(2);
    expect(readBrowserWorkspace("two").tabs[0].url).toBe("http://localhost:3000/task-two");
  });
});

describe("SimpleBrowser selected component context", () => {
  it("reviews sanitized selection and attaches annotation only on request", async () => {
    const onAttach = vi.fn();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/design" onAttachSelection={onAttach} />));
    const page = activePage();
    await click("Select page element"); expect(page.handle.action).toHaveBeenCalledWith("inspect");
    await emit(page, { selection });
    expect(container.querySelector('[aria-label="Selected page element"]')).not.toBeNull();
    expect(container.textContent).toContain("main > button:nth-of-type(1)");
    expect(container.textContent).not.toContain("private value");
    expect(onAttach).not.toHaveBeenCalled();
    const annotation = container.querySelector<HTMLTextAreaElement>('[aria-label="Selection annotation"]')!;
    await act(async () => { textareaSetter.call(annotation, "Make this button wider"); annotation.dispatchEvent(new Event("input", { bubbles: true })); });
    await click("Attach to prompt");
    expect(onAttach).toHaveBeenCalledTimes(1);
    expect(onAttach.mock.calls[0][0]).toMatchObject({ sessionId: "task", tabId: page.props.tabId, navigationId: 1, snippet: "<button>Save</button>" });
    expect(onAttach.mock.calls[0][0].annotations).toContainEqual({ kind: "note", text: "Make this button wider" });
    expect(onAttach.mock.calls[0][0].annotations).toContainEqual({ kind: "rectangle", points: [12, 30, 120, 32] });
    expect(container.querySelector('[aria-label="Selected page element"]')).toBeNull();
  });
  it("rejects attachment when a fresh snapshot reports navigation since selection", async () => {
    const onAttach = vi.fn();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/design" onAttachSelection={onAttach} />));
    const page = activePage(); await emit(page, { selection });
    page.current = { ...page.current, navigationId: 2, url: "http://localhost:3000/elsewhere", selection: null };
    await click("Attach to prompt");
    expect(onAttach).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("page changed");
    expect(container.querySelector('[aria-label="Selected page element"]')).toBeNull();
  });
  it.each(["cancel", "navigate", "unmount"] as const)("does not attach an awaited snapshot after %s", async interruption => {
    const onAttach = vi.fn();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/design" onAttachSelection={onAttach} />));
    const page = activePage(); await emit(page, { selection });
    const captured = page.current;
    let resolve!: (value: BrowserPageSnapshot) => void;
    vi.mocked(page.handle.snapshot).mockReturnValueOnce(new Promise<BrowserPageSnapshot>(done => { resolve = done; }));
    await click("Attach to prompt");
    if (interruption === "cancel") await key(address(), "Escape");
    else if (interruption === "navigate") await submit("localhost:3000/next");
    else await act(async () => root.render(null));
    await act(async () => resolve(captured));
    expect(onAttach).not.toHaveBeenCalled();
    expect(container.querySelector('[aria-label="Selected page element"]')).toBeNull();
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });
  it.each([{ loading: true }, { error: "The page failed" }])("rejects an attachment whose fresh page snapshot is unavailable: %j", async unavailable => {
    const onAttach = vi.fn();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/design" onAttachSelection={onAttach} />));
    const page = activePage(); await emit(page, { selection });
    page.current = { ...page.current, ...unavailable };
    await click("Attach to prompt");
    expect(onAttach).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("page changed");
  });
  it("invalidates selected context on navigation and supports Escape cancellation", async () => {
    const onInvalidate = vi.fn();
    await act(async () => root.render(<SimpleBrowser sessionId="task" initialUrl="localhost:3000/design" onInvalidateSelection={onInvalidate} />));
    const page = activePage(); await emit(page, { selection });
    await key(address(), "Escape");
    expect(container.querySelector('[aria-label="Selected page element"]')).toBeNull();
    expect(page.handle.action).toHaveBeenCalledWith("cancel_inspect");
    await emit(page, { selection });
    await emit(page, { navigationId: 2, url: "http://localhost:3000/next", selection: null });
    expect(container.querySelector('[aria-label="Selected page element"]')).toBeNull();
    expect(onInvalidate).toHaveBeenCalledWith(page.props.tabId, 2);
  });
});
