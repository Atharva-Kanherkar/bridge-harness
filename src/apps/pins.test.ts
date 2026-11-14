// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { MAX_PINS, parsePins, pinApp, PINS_KEY, readPins, resetPinsCache, subscribePins, togglePin, unpinApp } from "./pins";

beforeEach(() => { localStorage.clear(); resetPinsCache(); });
afterEach(() => { localStorage.clear(); resetPinsCache(); });

describe("parsePins", () => {
  it("drops unknown ids, repeats and anything past the limit", () => {
    expect(parsePins(JSON.stringify(["notes", "nope", "notes", "pulls", "usage", "memory"]))).toEqual(["notes", "pulls", "usage"]);
  });

  it("reads corrupt or foreign values as no pins", () => {
    expect(parsePins(null)).toEqual([]);
    expect(parsePins("{not json")).toEqual([]);
    expect(parsePins(JSON.stringify({ notes: true }))).toEqual([]);
  });
});

describe("pins", () => {
  it("pins in order and persists", () => {
    pinApp("notes");
    pinApp("pulls");
    expect(readPins()).toEqual(["notes", "pulls"]);
    expect(JSON.parse(localStorage.getItem(PINS_KEY)!)).toEqual(["notes", "pulls"]);
    resetPinsCache();
    expect(readPins()).toEqual(["notes", "pulls"]);
  });

  it("refuses a fourth pin and changes nothing", () => {
    pinApp("notes"); pinApp("pulls"); pinApp("usage");
    expect(MAX_PINS).toBe(3);
    expect(pinApp("memory")).toEqual({ ok: false, reason: "full" });
    expect(readPins()).toEqual(["notes", "pulls", "usage"]);
  });

  it("treats pinning a pinned app as done", () => {
    pinApp("notes");
    expect(pinApp("notes")).toEqual({ ok: true, pins: ["notes"] });
  });

  it("unpins and toggles", () => {
    pinApp("notes"); pinApp("pulls");
    expect(unpinApp("notes")).toEqual(["pulls"]);
    expect(togglePin("pulls")).toEqual({ ok: true, pins: [] });
    expect(togglePin("usage")).toEqual({ ok: true, pins: ["usage"] });
  });

  it("tells every subscriber about every change", () => {
    let heard = 0;
    const stop = subscribePins(() => { heard += 1; });
    pinApp("notes");
    unpinApp("notes");
    pinApp("notes");
    pinApp("notes");
    stop();
    pinApp("pulls");
    expect(heard).toBe(3);
  });
});

// Review fix: two web-client tabs share localStorage. Each used to cache its
// first read forever, so the second tab's pin overwrote the first tab's.
describe("pins across tabs", () => {
  // Another tab's write: storage changes, and this tab gets a storage event.
  const otherTabWrites = (pins: string[]) => {
    const newValue = JSON.stringify(pins);
    localStorage.setItem(PINS_KEY, newValue);
    window.dispatchEvent(new StorageEvent("storage", { key: PINS_KEY, newValue }));
  };

  it("builds on the other tab's pins instead of overwriting them", () => {
    expect(readPins()).toEqual([]); // this tab read before the other one pinned
    otherTabWrites(["notes"]);
    pinApp("usage");
    expect(JSON.parse(localStorage.getItem(PINS_KEY)!)).toEqual(["notes", "usage"]);
  });

  it("re-reads storage before a change even if the event has not arrived", () => {
    expect(readPins()).toEqual([]);
    localStorage.setItem(PINS_KEY, JSON.stringify(["notes"]));
    pinApp("usage");
    expect(readPins()).toEqual(["notes", "usage"]);
    localStorage.setItem(PINS_KEY, JSON.stringify(["notes", "usage", "memory"]));
    expect(pinApp("pulls")).toEqual({ ok: false, reason: "full" });
    expect(unpinApp("notes")).toEqual(["usage", "memory"]);
  });

  it("tells this tab's subscribers when the other tab pins", () => {
    expect(readPins()).toEqual([]);
    let heard = 0;
    const stop = subscribePins(() => { heard += 1; });
    otherTabWrites(["notes"]);
    expect(heard).toBe(1);
    expect(readPins()).toEqual(["notes"]);
    window.dispatchEvent(new StorageEvent("storage", { key: "something-else", newValue: "x" }));
    expect(heard).toBe(1);
    stop();
  });
});
