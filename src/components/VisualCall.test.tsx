// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import specs from "../../testing/fixtures/visualize/specs.json";
import type { ConversationItem } from "../conversation";
import { REFUSAL_PREFIX } from "../transcript/visual";
import { VisualCall } from "./VisualCall";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const spec = (specs as { name: string; spec: Record<string, unknown>; valid: boolean }[]).find(fixture => fixture.valid)!.spec;
const call = (overrides: Partial<ConversationItem> = {}, data: Record<string, unknown> = {}): ConversationItem => ({
  key: "k", identity: "i", type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", status: "completed",
  data: { name: "mcp__bridge__visualize", input: spec, ...data }, ...overrides,
});

let container: HTMLDivElement;
let root: Root;
let observers: { callback: IntersectionObserverCallback; node?: Element }[] = [];

beforeEach(() => {
  vi.spyOn(console, "debug").mockImplementation(() => undefined);
  observers = [];
  vi.stubGlobal("IntersectionObserver", class {
    callback: IntersectionObserverCallback;
    constructor(callback: IntersectionObserverCallback) {
      this.callback = callback;
      observers.push({ callback });
    }
    observe(node: Element) { observers[observers.length - 1].node = node; }
    disconnect() {}
    unobserve() {}
    takeRecords() { return []; }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const draw = (item: ConversationItem, turnActive = false) => act(() => root.render(<VisualCall item={item} turnActive={turnActive} />));
const scroll = (visible: boolean) => act(() => {
  for (const { callback, node } of observers) callback([{ isIntersecting: visible, target: node } as IntersectionObserverEntry], {} as IntersectionObserver);
});

describe("VisualCall", () => {
  it("a drawn call is a card with the view frame once it is near the viewport", () => {
    draw(call());
    expect(container.querySelector('[data-visual-state="drawn"]')).not.toBeNull();
    expect(container.textContent).toContain(String(spec.title));
    expect(container.querySelector("iframe")).toBeNull();
    scroll(true);
    expect(container.querySelector("iframe[data-mcp-app]")).not.toBeNull();
  });

  it("an_offscreen_card_unmounts_its_frame_and_keeps_its_height", () => {
    draw(call());
    scroll(true);
    expect(container.querySelector("iframe")).not.toBeNull();
    scroll(false);
    expect(container.querySelector("iframe")).toBeNull();
    const placeholder = container.querySelector('[data-visual-state="drawn"] > div[aria-hidden="true"]') as HTMLElement;
    expect(placeholder.style.height).toBe("320px");
  });

  it("expand_opens_fullscreen_and_escape_closes", async () => {
    draw(call());
    const expand = container.querySelector('[aria-label="Expand visual"]') as HTMLButtonElement;
    act(() => expand.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog!.querySelector("iframe[data-mcp-app]")).not.toBeNull();
    act(() => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
    expect(document.querySelector('[role="dialog"] iframe[data-mcp-app]')).toBeNull();
  });

  it("an_invalid_spec_shows_a_quiet_error_row", () => {
    draw(call({}, { input: { ...spec, version: 9 } }));
    expect(container.querySelector('[data-visual-state="refused"]')).not.toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.textContent).toContain("could not be drawn");
  });

  it("a refused call is one quiet line that opens to the reasons", () => {
    const refusal = `${REFUSAL_PREFIX} Fix every item below and call visualize once more:\n- version: must be 1`;
    draw(call({ status: "failed" }, { aggregatedOutput: refusal }));
    expect(container.textContent).toContain("Visual not drawn");
    expect(container.textContent).not.toContain("must be 1");
    act(() => (container.querySelector("button") as HTMLButtonElement).click());
    expect(container.textContent).toContain("- version: must be 1");
  });

  it("a call still being written is a placeholder, and a stopped one says so", () => {
    draw(call({ status: "inProgress" }), true);
    expect(container.querySelector('[data-visual-state="drawing"]')?.textContent).toContain(String(spec.title));
    draw(call({ status: "inProgress" }), false);
    expect(container.textContent).toContain("Visual not finished");
  });
});
