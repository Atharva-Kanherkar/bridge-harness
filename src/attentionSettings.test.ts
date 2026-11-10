// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ATTENTION_SETTINGS_KEY,
  DEFAULT_ATTENTION_SETTINGS,
  isQuiet,
  normalizeAttentionSettings,
  readAttentionSettings,
  writeAttentionSettings,
} from "./attentionSettings";

const at = (hours: number, minutes = 0) => new Date(2026, 10, 9, hours, minutes);

describe("attention settings", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => { store.set(key, value); }, removeItem: (key: string) => { store.delete(key); } },
    });
  });
  afterEach(() => vi.restoreAllMocks());

  it("starts with sound only for requests", () => {
    expect(readAttentionSettings()).toEqual(DEFAULT_ATTENTION_SETTINGS);
    expect(DEFAULT_ATTENTION_SETTINGS.kinds.needsYou.sound).toBe(true);
    expect(DEFAULT_ATTENTION_SETTINGS.kinds.escalation.sound).toBe(true);
    expect(DEFAULT_ATTENTION_SETTINGS.kinds.finished.sound).toBe(false);
    expect(DEFAULT_ATTENTION_SETTINGS.kinds.failed.sound).toBe(false);
  });

  it("reads back a write and announces it", () => {
    const heard = vi.fn();
    window.addEventListener("bridge:attention-settings", heard);
    const next = structuredClone(DEFAULT_ATTENTION_SETTINGS);
    next.kinds.finished.toast = false;
    next.quietHours = { enabled: true, start: "23:30", end: "07:15" };
    writeAttentionSettings(next);
    expect(readAttentionSettings()).toEqual(next);
    expect(heard).toHaveBeenCalledTimes(1);
    window.removeEventListener("bridge:attention-settings", heard);
  });

  it("falls back field by field on partial or corrupt values", () => {
    expect(normalizeAttentionSettings({ kinds: { finished: { toast: false, sound: "yes" } }, quietHours: { start: "25:00", enabled: true } })).toEqual({
      kinds: { ...DEFAULT_ATTENTION_SETTINGS.kinds, finished: { toast: false, banner: true, sound: false } },
      quietHours: { enabled: true, start: "22:00", end: "08:00" },
    });
    localStorage.setItem(ATTENTION_SETTINGS_KEY, "{not json");
    expect(readAttentionSettings()).toEqual(DEFAULT_ATTENTION_SETTINGS);
  });

  it("knows quiet hours, across midnight too", () => {
    const overnight = { enabled: true, start: "22:00", end: "08:00" };
    expect(isQuiet(overnight, at(23))).toBe(true);
    expect(isQuiet(overnight, at(3))).toBe(true);
    expect(isQuiet(overnight, at(8))).toBe(false);
    expect(isQuiet(overnight, at(12))).toBe(false);
    const lunch = { enabled: true, start: "12:00", end: "13:30" };
    expect(isQuiet(lunch, at(12, 45))).toBe(true);
    expect(isQuiet(lunch, at(13, 30))).toBe(false);
    expect(isQuiet({ ...overnight, enabled: false }, at(23))).toBe(false);
  });
});
