// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { importLegacyPins, parsePins, PIN_LIMIT_MESSAGE, PINS_KEY, setPinned, togglePin } from "./pins";
import { pinsOf, resetAppsStore } from "./store";
import { resetMockApps } from "../appsMock";
import { bridgeApi } from "../api";

const corePins = async () => pinsOf((await bridgeApi.listApps()).apps);

beforeEach(() => { localStorage.clear(); resetMockApps(); resetAppsStore(); });
afterEach(() => { localStorage.clear(); resetMockApps(); resetAppsStore(); });

describe("parsePins", () => {
  it("keeps strings in order, drops repeats and anything past three", () => {
    expect(parsePins(JSON.stringify(["notes", 4, "notes", "pulls", "usage", "memory"]))).toEqual(["notes", "pulls", "usage"]);
  });

  it("reads corrupt or foreign values as no pins", () => {
    expect(parsePins(null)).toEqual([]);
    expect(parsePins("{not json")).toEqual([]);
    expect(parsePins(JSON.stringify({ notes: true }))).toEqual([]);
  });
});

describe("pins in core", () => {
  it("pin, unpin and toggle go through core", async () => {
    expect(await setPinned("notes", true)).toEqual({ ok: true });
    expect(await setPinned("pulls", true)).toEqual({ ok: true });
    expect(await corePins()).toEqual(["notes", "pulls"]);
    expect(await togglePin("notes", ["notes", "pulls"])).toEqual({ ok: true });
    expect(await corePins()).toEqual(["pulls"]);
  });

  it("carries core's refusal of a fourth pin", async () => {
    for (const id of ["notes", "pulls", "usage"] as const) await setPinned(id, true);
    expect(await setPinned("memory", true)).toEqual({ ok: false, message: PIN_LIMIT_MESSAGE });
    expect(await corePins()).toEqual(["notes", "pulls", "usage"]);
  });
});

describe("the slice 1 import", () => {
  it("moves local pins into core in order, once, and forgets the key", async () => {
    localStorage.setItem(PINS_KEY, JSON.stringify(["usage", "nope", "notes"]));
    await importLegacyPins();
    expect(await corePins()).toEqual(["usage", "notes"]);
    expect(localStorage.getItem(PINS_KEY)).toBeNull();
    await setPinned("usage", false);
    await importLegacyPins();
    expect(await corePins(), "the import does not run a second time").toEqual(["notes"]);
  });

  it("stops at a full sidebar without failing", async () => {
    for (const id of ["notes", "pulls", "usage"] as const) await setPinned(id, true);
    localStorage.setItem(PINS_KEY, JSON.stringify(["memory"]));
    await importLegacyPins();
    expect(await corePins()).toEqual(["notes", "pulls", "usage"]);
    expect(localStorage.getItem(PINS_KEY)).toBeNull();
  });
});
