// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

type Snapshot = {
  revision: number; cancelled?: boolean; shortcut?: string; historyAction?: string; popupUrl?: string;
  selection?: { selector: string; snippet: string; bounds: { x: number; y: number; width: number; height: number } };
};
type PageApi = { setInspect(enabled: boolean): void; snapshot(): Snapshot };
type PageEvent = {
  isTrusted: boolean; key?: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean;
  composedPath(): Element[]; preventDefault(): void; stopImmediatePropagation(): void;
};
const listeners = new Map<string, Array<(event: PageEvent) => void>>();
let api: PageApi;
const originalPush = history.pushState;
const originalReplace = history.replaceState;

beforeAll(() => {
  // jsdom cannot create trusted input. Capture browser-registered callbacks and
  // exercise them with explicit trust metadata, without rewriting the shipped
  // script or weakening its guard. Synthetic-event rejection is covered too.
  const pageWindow: { top?: unknown; __bridgeBrowser?: PageApi } = {};
  pageWindow.top = pageWindow;
  const script = readFileSync(resolve(process.cwd(), "src-tauri/src/browser_page.js"), "utf8");
  new Function("window", "addEventListener", script)(pageWindow, (type: string, listener: (event: PageEvent) => void) => {
    listeners.set(type, [...(listeners.get(type) ?? []), listener]);
  });
  api = pageWindow.__bridgeBrowser!;
});

beforeEach(() => {
  document.body.innerHTML = "";
  api.setInspect(false);
  api.snapshot();
});

afterAll(() => { history.pushState = originalPush; history.replaceState = originalReplace; });

function element(markup: string): HTMLElement {
  document.body.innerHTML = markup;
  const node = document.body.firstElementChild as HTMLElement;
  node.getBoundingClientRect = () => ({ x: 10, y: 20, width: 120, height: 40, top: 20, left: 10, right: 130, bottom: 60, toJSON() {} });
  return node;
}
function input(type: string, node: Element, overrides: Partial<PageEvent> = {}) {
  const event: PageEvent = { isTrusted: true, composedPath: () => [node], preventDefault: vi.fn(), stopImmediatePropagation: vi.fn(), ...overrides };
  for (const listener of listeners.get(type) ?? []) listener(event);
  return event;
}

describe("native injected browser picker", () => {
  it("highlights the hovered element, intercepts selection clicks, and drains selection once", () => {
    const node = element('<button>Save</button>');
    api.setInspect(true);
    input("pointermove", node);
    const overlay = document.querySelector<HTMLElement>("[data-bridge-picker-overlay]")!;
    expect(overlay.style.left).toBe("10px");
    expect(overlay.style.width).toBe("120px");
    for (const type of ["pointerdown", "pointerup", "mousedown", "mouseup"]) {
      const pointer = input(type, node);
      expect(pointer.preventDefault).toHaveBeenCalledOnce();
      expect(pointer.stopImmediatePropagation).toHaveBeenCalledOnce();
    }
    const event = input("click", node);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
    expect(api.snapshot().selection).toMatchObject({ selector: "html > body > button", snippet: "<button>Save</button>", bounds: { x: 10, y: 20, width: 120, height: 40 } });
    expect(api.snapshot().selection).toBeUndefined();
    expect(document.querySelector("[data-bridge-picker-overlay]")).toBeNull();
  });

  it("omits editable, sensitive, and hidden descendants from selected containers", () => {
    const node = element('<section>Visible<input value="typed-secret"><textarea>text-secret</textarea><div contenteditable="true">edit-secret</div><p data-private>private-secret</p><div style="display:none"><span>hidden-secret</span></div><script>script-secret</script><button>Save</button></section>');
    api.setInspect(true);
    input("click", node);
    const snippet = api.snapshot().selection!.snippet;
    expect(snippet).toContain("Visible Save");
    expect(snippet).not.toContain("secret");
    expect(snippet).not.toContain("value=");
  });

  it("invalidates a selected element on SPA navigation and reports the navigation kind", () => {
    const node = element('<button>Save</button>');
    api.setInspect(true);
    input("click", node);
    const prior = api.snapshot().revision;
    api.setInspect(true);
    input("pointermove", node);
    history.pushState({}, "", "#route");
    const changed = api.snapshot();
    expect(changed.revision).toBeGreaterThan(prior);
    expect(changed.historyAction).toBe("push");
    expect(changed.selection).toBeUndefined();
    expect(changed.cancelled).toBe(true);
    expect(document.querySelector("[data-bridge-picker-overlay]")).toBeNull();
  });

  it("cancels with Escape and drains keyboard shortcuts exactly once", () => {
    const node = element('<button>Save</button>');
    api.setInspect(true);
    const cancelled = input("keydown", node, { key: "Escape" });
    expect(cancelled.preventDefault).toHaveBeenCalledOnce();
    expect(api.snapshot().cancelled).toBe(true);
    expect(api.snapshot().cancelled).toBe(false);
    input("keydown", node, { key: "t", metaKey: true });
    expect(api.snapshot().shortcut).toBe("new_tab");
    expect(api.snapshot().shortcut).toBeUndefined();
    input("keydown", node, { key: "t", metaKey: true, shiftKey: true });
    expect(api.snapshot().shortcut).toBe("reopen_tab");
  });

  it("does not accept page-generated clicks or shortcuts as user intent", () => {
    const node = element('<button>Save</button>');
    api.setInspect(true);
    input("click", node, { isTrusted: false });
    input("keydown", node, { key: "w", ctrlKey: true, isTrusted: false });
    expect(api.snapshot()).toMatchObject({ selection: undefined, shortcut: undefined });
  });

  it("bounds hostile page text before returning it to the host", () => {
    const node = element(`<section>${"<span>large &amp; repeated content</span>".repeat(1_000)}</section>`);
    api.setInspect(true);
    input("click", node);
    const selected = api.snapshot().selection!;
    expect(selected.snippet.length).toBeLessThanOrEqual(1_600);
    expect(selected.selector.length).toBeLessThanOrEqual(1_024);
  });
  it("queues a trusted new-tab link once and suppresses the native popup", () => {
    const node = element('<a href="http://localhost:3000/second" target="_blank">Open another page</a>');
    const event = input("click", node);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopImmediatePropagation).toHaveBeenCalledOnce();
    expect(api.snapshot().popupUrl).toBe("http://localhost:3000/second");
    expect(api.snapshot().popupUrl).toBeUndefined();
  });

  it("ignores synthetic new-tab clicks and selects inspected links without opening them", () => {
    const node = element('<a href="http://localhost:3000/second" target="_blank">Open another page</a>');
    input("click", node, { isTrusted: false });
    expect(api.snapshot()).toMatchObject({ popupUrl: undefined, selection: undefined });
    api.setInspect(true);
    input("click", node);
    expect(api.snapshot()).toMatchObject({ popupUrl: undefined, selection: { selector: "html > body > a" } });
  });

  it("rejects unsafe new-tab URLs and excludes download links", () => {
    for (const href of ["javascript:alert(1)", "file:///private/data", "data:text/html,hello", "https://user:password@example.com/"]) {
      const node = element(`<a href="${href}" target="_blank">Open</a>`);
      input("click", node);
      expect(api.snapshot().popupUrl).toBeUndefined();
    }
    const download = element('<a href="https://example.com/file" target="_blank" download>Download</a>');
    input("click", download);
    expect(api.snapshot().popupUrl).toBeUndefined();
  });

});
