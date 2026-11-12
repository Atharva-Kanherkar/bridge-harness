import { useSyncExternalStore } from "react";

// How each attention kind may reach the person, per client. Kept with the
// other General preferences in localStorage; every window hears a change.

export const ATTENTION_KINDS = ["needsYou", "posted", "escalation", "failed", "finished"] as const;
export type AttentionSettingKind = (typeof ATTENTION_KINDS)[number];
export type Channels = { toast: boolean; banner: boolean; sound: boolean };
export type QuietHours = { enabled: boolean; start: string; end: string };
export type AttentionSettings = {
  kinds: Record<AttentionSettingKind, Channels>;
  quietHours: QuietHours;
};

export const ATTENTION_SETTINGS_KEY = "bridge.attention.settings";
const CHANGE_EVENT = "bridge:attention-settings";

/** A sound for every finished turn is noise; requests are worth hearing. */
export const DEFAULT_ATTENTION_SETTINGS: AttentionSettings = {
  kinds: {
    needsYou: { toast: true, banner: true, sound: true },
    posted: { toast: true, banner: true, sound: true },
    escalation: { toast: true, banner: true, sound: true },
    failed: { toast: true, banner: true, sound: false },
    finished: { toast: true, banner: true, sound: false },
  },
  quietHours: { enabled: false, start: "22:00", end: "08:00" },
};

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;
const bool = (value: unknown, fallback: boolean) => (typeof value === "boolean" ? value : fallback);
const time = (value: unknown, fallback: string) => (typeof value === "string" && TIME.test(value) ? value : fallback);

/** Field by field, so one bad value never resets the rest. */
export function normalizeAttentionSettings(raw: unknown): AttentionSettings {
  const value = (raw && typeof raw === "object" ? raw : {}) as Partial<Record<string, unknown>>;
  const kinds = (value.kinds && typeof value.kinds === "object" ? value.kinds : {}) as Record<string, unknown>;
  const quiet = (value.quietHours && typeof value.quietHours === "object" ? value.quietHours : {}) as Record<string, unknown>;
  const defaults = DEFAULT_ATTENTION_SETTINGS;
  return {
    kinds: Object.fromEntries(ATTENTION_KINDS.map(kind => {
      const stored = (kinds[kind] && typeof kinds[kind] === "object" ? kinds[kind] : {}) as Record<string, unknown>;
      const fallback = defaults.kinds[kind];
      return [kind, { toast: bool(stored.toast, fallback.toast), banner: bool(stored.banner, fallback.banner), sound: bool(stored.sound, fallback.sound) }];
    })) as AttentionSettings["kinds"],
    quietHours: {
      enabled: bool(quiet.enabled, defaults.quietHours.enabled),
      start: time(quiet.start, defaults.quietHours.start),
      end: time(quiet.end, defaults.quietHours.end),
    },
  };
}

let cachedRaw: string | null | undefined;
let cached = DEFAULT_ATTENTION_SETTINGS;

export function readAttentionSettings(): AttentionSettings {
  let raw: string | null = null;
  try { raw = localStorage.getItem(ATTENTION_SETTINGS_KEY); } catch { return DEFAULT_ATTENTION_SETTINGS; }
  // useSyncExternalStore needs a stable snapshot between changes.
  if (raw === cachedRaw) return cached;
  cachedRaw = raw;
  try { cached = normalizeAttentionSettings(raw ? JSON.parse(raw) : null); } catch { cached = DEFAULT_ATTENTION_SETTINGS; }
  return cached;
}

export function writeAttentionSettings(settings: AttentionSettings): void {
  localStorage.setItem(ATTENTION_SETTINGS_KEY, JSON.stringify(normalizeAttentionSettings(settings)));
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, onChange);
  window.addEventListener("storage", onChange);
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onChange);
  };
}

export function useAttentionSettings(): AttentionSettings {
  return useSyncExternalStore(subscribe, readAttentionSettings, () => DEFAULT_ATTENTION_SETTINGS);
}

const minutes = (value: string) => Number(value.slice(0, 2)) * 60 + Number(value.slice(3, 5));

/** Whether `now` falls in quiet hours. A window may cross midnight. */
export function isQuiet(quiet: QuietHours, now: Date): boolean {
  if (!quiet.enabled) return false;
  const start = minutes(quiet.start);
  const end = minutes(quiet.end);
  const at = now.getHours() * 60 + now.getMinutes();
  if (start === end) return false;
  return start < end ? at >= start && at < end : at >= start || at < end;
}
