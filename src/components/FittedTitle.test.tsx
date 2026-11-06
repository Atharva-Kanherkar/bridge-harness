// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { addedWords, FittedTitle, pickRung } from "./FittedTitle";

// Seven pixels a character: easy to reason about, and what the fake canvas below returns.
const byChars = (text: string) => text.length * 7;

const LADDER = [
  "WAL",
  "WAL growth",
  "SQLite WAL growth",
  "SQLite WAL growth in bridged",
  "Why the SQLite WAL keeps growing in bridged",
];

describe("pickRung", () => {
  it("takes the longest rung that fits and the narrowest when none does", () => {
    expect(pickRung(LADDER, 1000, byChars)).toBe(LADDER[4]);
    expect(pickRung(LADDER, 17 * 7, byChars)).toBe("SQLite WAL growth");
    expect(pickRung(LADDER, 17 * 7 - 1, byChars)).toBe("WAL growth");
    expect(pickRung(LADDER, 5, byChars)).toBe("WAL");
  });

  it("goes by measured width, not by the order rungs arrive in", () => {
    expect(pickRung([...LADDER].reverse(), 12 * 7, byChars)).toBe("WAL growth");
  });

  it("gives up when the text cannot be measured", () => {
    expect(pickRung(LADDER, 200, () => null)).toBeNull();
  });
});

describe("addedWords", () => {
  it("marks only the words the new rung brings", () => {
    expect(addedWords("SQLite WAL growth", "SQLite WAL growth in bridged")).toEqual([false, false, false, true, true]);
    // shrinking keeps the shared words still and brings in the new phrasing.
    expect(addedWords("Why the SQLite WAL keeps growing", "WAL growth")).toEqual([false, true]);
    // case and punctuation do not make a word new; a repeat does.
    expect(addedWords("Fix: the rail", "fix the rail, the rail")).toEqual([false, false, false, true, true]);
  });
});

describe("FittedTitle", () => {
  let container: HTMLDivElement;
  let root: Root;
  let width = 0;
  let resized: ((entries: { target: Element; contentRect: { width: number } }[]) => void) | undefined;
  const observed = new Set<Element>();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.stubGlobal("OffscreenCanvas", class {
      getContext() {
        return { font: "", measureText: (text: string) => ({ width: byChars(text) }) };
      }
    });
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: typeof resized) { resized = callback; }
      observe(element: Element) { observed.add(element); }
      unobserve(element: Element) { observed.delete(element); }
      disconnect() {}
    });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({ width } as DOMRect));
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  const resize = (next: number) => act(() => {
    width = next;
    resized?.([...observed].map(target => ({ target, contentRect: { width: next } })));
  });

  it("mounts already fitted, without animating", () => {
    width = 17 * 7;
    act(() => root.render(<FittedTitle title="SQLite WAL growth" ladder={LADDER} />));
    expect(container.textContent).toBe("SQLite WAL growth");
    expect(container.querySelector(".title-word-in")).toBeNull();
  });

  it("rewords on resize and animates only the words the new rung adds", () => {
    width = 17 * 7;
    act(() => root.render(<FittedTitle title="SQLite WAL growth" ladder={LADDER} />));
    resize(400);
    expect(container.textContent).toBe("Why the SQLite WAL keeps growing in bridged");
    expect([...container.querySelectorAll(".title-word-in")].map(word => word.textContent)).toEqual(["Why", "the", "keeps", "growing", "in", "bridged"]);
    resize(80);
    expect(container.textContent).toBe("WAL growth");
    expect([...container.querySelectorAll(".title-word-in")].map(word => word.textContent)).toEqual(["growth"]);
    // the same width again changes nothing.
    resize(80);
    expect(container.textContent).toBe("WAL growth");
  });

  it("animates a ladder that arrives for a row already on screen", () => {
    width = 400;
    act(() => root.render(<FittedTitle title="New chat" />));
    expect(container.textContent).toBe("New chat");
    act(() => root.render(<FittedTitle title="SQLite WAL growth" ladder={LADDER} />));
    expect(container.textContent).toBe(LADDER[4]);
    expect(container.querySelectorAll(".title-word-in").length).toBeGreaterThan(0);
  });

  it("shows a title with no ladder exactly as written, and stops watching it", () => {
    width = 10;
    act(() => root.render(<FittedTitle title="Chosen by hand" ladder={["Chosen by hand"]} />));
    expect(container.textContent).toBe("Chosen by hand");
    expect(observed.size).toBe(0);
  });

  it("falls back to the title where the webview cannot measure text", async () => {
    vi.stubGlobal("OffscreenCanvas", undefined);
    vi.resetModules();
    const { FittedTitle: Unmeasured } = await import("./FittedTitle");
    width = 400;
    act(() => root.render(<Unmeasured title="SQLite WAL growth" ladder={LADDER} />));
    expect(container.textContent).toBe("SQLite WAL growth");
  });
});
