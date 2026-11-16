// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BrowserPage } from "./BrowserPage";

const harness = vi.hoisted(() => ({
  commands: [] as Array<{ command: string; args: Record<string, unknown> }>,
  observers: { resize: [] as Array<() => void> },
}));
vi.mock("../browserRuntime", () => ({
  hasNativeBrowser: () => true,
  browserCommand: (command: string, _sessionId: string, _tabId: string, args: Record<string, unknown> = {}) => {
    harness.commands.push({ command, args });
    if (command === "snapshot") {
      return Promise.resolve({ url: "https://example.com/", title: "", canGoBack: false, canGoForward: false, loading: false, navigationId: 1 });
    }
    return Promise.resolve();
  },
  browserViewportVisible: (_element: HTMLElement, visible: boolean) => visible,
  registerBrowserPageValidator: () => () => {},
}));

class ResizeObserverStub {
  static instances: ResizeObserverStub[] = [];
  callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    ResizeObserverStub.instances.push(this);
  }
  observe() {}
  unobserve() {}
  disconnect() { const index = ResizeObserverStub.instances.indexOf(this); if (index >= 0) ResizeObserverStub.instances.splice(index, 1); }
}
class MutationObserverStub {
  observe() {}
  disconnect() {}
  takeRecords() { return []; }
}

let container: HTMLDivElement;
let root: Root;
const render = (initialUrl = "https://example.com/") => {
  const page = <BrowserPage sessionId="task" tabId="tab" initialUrl={initialUrl} visible onSnapshot={() => {}} onError={() => {}} />;
  act(() => { root.render(page); });
};
/** Point the rendered viewport host at a fake DOM rect and fire the observer. */
/** Point the rendered viewport host at a fake DOM rect and fire the observer.
 * The coalesced layout task settles in microtasks plus a timer, so flush hard. */
const flushLayout = async () => {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
};
const reportHostRect = async (rect: { left: number; top: number; right: number; bottom: number }) => {
  const host = container.querySelector<HTMLElement>("[data-browser-viewport]");
  if (!host) throw new Error("Missing browser viewport host");
  const width = rect.right - rect.left;
  const height = rect.bottom - rect.top;
  vi.spyOn(host, "getBoundingClientRect").mockReturnValue({ ...rect, width, height, x: rect.left, y: rect.top, toJSON: () => rect } as DOMRect);
  const observer = ResizeObserverStub.instances.at(-1);
  if (!observer) throw new Error("Missing resize observer");
  await act(async () => { observer.callback([], observer as unknown as ResizeObserver); });
  await flushLayout();
};
const lastLayout = () => {
  const layout = [...harness.commands].reverse().find(entry => entry.command === "layout");
  if (!layout) throw new Error("No layout command was issued");
  return layout.args;
};

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  vi.stubGlobal("MutationObserver", MutationObserverStub);
  vi.stubGlobal("innerWidth", 800);
  vi.stubGlobal("innerHeight", 600);
  harness.commands.length = 0;
  ResizeObserverStub.instances.length = 0;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  render();
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("BrowserPage native viewport bounds", () => {
  it("clamps a host rect that extends past the window", async () => {
    await reportHostRect({ left: 50, top: -20, right: 900, bottom: 700 });
    // Window is 800×600: the frame may never exceed it, and never rise above
    // its top edge — the old Math.max(1, …) floor is gone as well.
    expect(lastLayout()).toMatchObject({ x: 50, y: 0, width: 750, height: 600, visible: true });
  });

  it("keeps a fully in-window rect untouched", async () => {
    await reportHostRect({ left: 400, top: 120, right: 800, bottom: 500 });
    expect(lastLayout()).toMatchObject({ x: 400, y: 120, width: 400, height: 380, visible: true });
  });

  it("reports a collapsed pane as invisible instead of a one-pixel view", async () => {
    await reportHostRect({ left: 0, top: 0, right: 0, bottom: 0 });
    expect(lastLayout()).toMatchObject({ x: 0, y: 0, width: 0, height: 0, visible: false });
  });
});
