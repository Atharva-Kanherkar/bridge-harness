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
