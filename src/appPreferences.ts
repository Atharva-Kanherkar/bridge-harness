// App preferences follow the core. Theme, skin, the effort picker's style and
// transcript display are drawn by this app, but the core keeps their record so
// a chat (through `settings_change`), another window or the web client can
// change them. localStorage stays the first-paint cache: every value the core
// sends is written there with the same writers Settings uses, and the same
// in-window events fire, so every mounted consumer repaints without a reload.

import type { AppPreferences, AppPreferenceValues } from "./protocol/generated/protocol";
import {
  EFFORT_SELECTOR_STORAGE_KEY, isEffortSelectorStyle, isThemePreference, isThemeSkin, SKIN_STORAGE_KEY, THEME_EVENT,
  THEME_STORAGE_KEY, writeEffortSelectorStyle, writeThemePreference, writeThemeSkin,
} from "./theme";
import {
  AUTO_EXPAND_EDIT_ACTIVITY_STORAGE_KEY, SHOW_THINKING_EVENT, SHOW_THINKING_STORAGE_KEY, writeAutoExpandEditActivity, writeShowThinking,
} from "./transcriptSettings";

type Key = keyof AppPreferenceValues;
const THEME_KEYS: Key[] = ["theme", "skin", "effortSelector"];
const TRANSCRIPT_KEYS: Key[] = ["showThinking", "autoExpandEdits"];
const KEYS: Key[] = [...THEME_KEYS, ...TRANSCRIPT_KEYS];

export type AppPreferencesApi = {
  getAppPreferences: () => Promise<AppPreferences>;
  saveAppPreferences: (values: AppPreferenceValues) => Promise<AppPreferences>;
  onAppPreferencesChanged: (handler: () => void) => Promise<() => void>;
};

type Store = Pick<Storage, "getItem" | "setItem">;

function raw(storage: Pick<Storage, "getItem">, key: string): string | null {
  try {
    return storage.getItem(key);
  } catch {
    return null;
  }
}

function flag(text: string | null): boolean | undefined {
  return text === "true" ? true : text === "false" ? false : undefined;
}

/** The values this window has explicitly stored. A key the person never set is
 *  absent, so a built-in default is never promoted into a choice. */
export function readLocalPreferences(storage: Pick<Storage, "getItem"> = localStorage): AppPreferenceValues {
  const values: AppPreferenceValues = {};
  const theme = raw(storage, THEME_STORAGE_KEY);
  if (isThemePreference(theme)) values.theme = theme;
  const skin = raw(storage, SKIN_STORAGE_KEY);
  if (isThemeSkin(skin)) values.skin = skin;
  const effortSelector = raw(storage, EFFORT_SELECTOR_STORAGE_KEY);
  if (isEffortSelectorStyle(effortSelector)) values.effortSelector = effortSelector;
  const showThinking = flag(raw(storage, SHOW_THINKING_STORAGE_KEY));
  if (showThinking !== undefined) values.showThinking = showThinking;
  const autoExpandEdits = flag(raw(storage, AUTO_EXPAND_EDIT_ACTIVITY_STORAGE_KEY));
  if (autoExpandEdits !== undefined) values.autoExpandEdits = autoExpandEdits;
  return values;
}

/** The keys `next` sets to something other than what `base` has. */
export function changedKeys(next: AppPreferenceValues, base: AppPreferenceValues): AppPreferenceValues {
  const out: Record<string, unknown> = {};
  for (const key of KEYS) {
    if (next[key] !== undefined && next[key] !== null && next[key] !== base[key]) out[key] = next[key];
  }
  return out as AppPreferenceValues;
}

const isEmpty = (values: AppPreferenceValues) => Object.keys(values).length === 0;

/** Write `values` to the cache and tell this window's consumers. */
export function applyLocalPreferences(values: AppPreferenceValues, storage: Store = localStorage, target: Pick<Window, "dispatchEvent"> = window): void {
  if (values.theme) writeThemePreference(values.theme, storage);
  if (values.skin) writeThemeSkin(values.skin, storage);
  if (values.effortSelector) writeEffortSelectorStyle(values.effortSelector, storage);
  if (typeof values.showThinking === "boolean") writeShowThinking(values.showThinking, storage);
  if (typeof values.autoExpandEdits === "boolean") writeAutoExpandEditActivity(values.autoExpandEdits, storage);
  if (THEME_KEYS.some(key => values[key] != null)) target.dispatchEvent(new Event(THEME_EVENT));
  if (TRANSCRIPT_KEYS.some(key => values[key] != null)) target.dispatchEvent(new Event(SHOW_THINKING_EVENT));
}

/**
 * Keep this window and the core in step. On start the core's values win; a key
 * only this window has is seeded into the core once. After that a change made
 * here is saved, and a change made anywhere else arrives as a hint and is
 * applied. Returns a stop function.
 */
export function startAppPreferenceSync(api: AppPreferencesApi, storage: Store = localStorage, target: Pick<Window, "addEventListener" | "removeEventListener" | "dispatchEvent"> = window): () => void {
  let stopped = false;
  /** What this window's consumers show. Not the shared cache: another window
   *  on the same origin may already have written the value this one has yet
   *  to paint, and only this window's own events repaint it. */
  let rendered: AppPreferenceValues = readLocalPreferences(storage);
  /** Keys saved from here and not yet answered, so a stale read cannot undo them. */
  const pending = new Set<Key>();

  const save = (values: AppPreferenceValues) => {
    const keys = Object.keys(values) as Key[];
    keys.forEach(key => pending.add(key));
    rendered = { ...rendered, ...values };
    void api.saveAppPreferences(values)
      .catch(() => undefined)
      .finally(() => keys.forEach(key => pending.delete(key)));
  };

  const pull = async (seed: boolean) => {
    let record: AppPreferences;
    try {
      record = await api.getAppPreferences();
    } catch {
      return;
    }
    if (stopped) return;
    const core = record.values;
    const incoming = changedKeys(core, rendered);
    for (const key of pending) delete (incoming as Record<string, unknown>)[key];
    const missing: Record<string, unknown> = {};
    if (seed) for (const key of KEYS) if (core[key] == null && rendered[key] != null) missing[key] = rendered[key];
    // Record what is about to be painted first, so the events `apply` fires
    // are not mistaken for local edits and saved back.
    rendered = { ...rendered, ...incoming };
    if (!isEmpty(incoming)) applyLocalPreferences(incoming, storage, target);
    if (!isEmpty(missing)) save(missing as AppPreferenceValues);
  };

  const onLocalChange = () => {
    if (stopped) return;
    const outgoing = changedKeys(readLocalPreferences(storage), rendered);
    if (!isEmpty(outgoing)) save(outgoing);
  };

  target.addEventListener(THEME_EVENT, onLocalChange);
  target.addEventListener(SHOW_THINKING_EVENT, onLocalChange);
  let unlisten: (() => void) | undefined;
  void api.onAppPreferencesChanged(() => { void pull(false); }).then(off => {
    if (stopped) off();
    else unlisten = off;
  });
  void pull(true);

  return () => {
    stopped = true;
    target.removeEventListener(THEME_EVENT, onLocalChange);
    target.removeEventListener(SHOW_THINKING_EVENT, onLocalChange);
    unlisten?.();
  };
}
