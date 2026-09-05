// Theme preference: the app follows macOS appearance unless the user pins a
// mode. Both modes are first-class renderings of the same token names, so the
// only thing that changes at runtime is the `dark` class on <html>.

import { useCallback, useEffect, useState } from "react";

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

/**
 * The skin is orthogonal to the light/dark mode: it chooses the *character* of
 * the chrome, not its brightness. `graphite` is the opaque pure-black shell;
 * `vibrancy` is the translucent, cursor-style shell that lets AppKit wash the
 * desktop wallpaper through the grounds. Both render every token — only the
 * `data-skin` attribute on <html> changes at runtime. This is the seed of the
 * user-customisable theme pack; new skins slot in by extending this union.
 */
export type ThemeSkin = "graphite" | "vibrancy";

export const THEME_STORAGE_KEY = "bridge.theme";
export const SKIN_STORAGE_KEY = "bridge.skin";

/** The shell stays exactly as it ships until the user opts into another skin. */
export const DEFAULT_SKIN: ThemeSkin = "graphite";

/**
 * Window background for the `theme-color` meta, kept in sync with `--background`
 * in index.css. Dark depends on the skin: graphite grounds are true black,
 * vibrancy lifts them to near-black so the wallpaper wash has a base to tint.
 */
function themeColorFor(resolved: ResolvedTheme, skin: ThemeSkin): string {
  if (resolved === "light") return "#fafaf9";
  return skin === "vibrancy" ? "#111111" : "#000000";
}

/** Reads the skin already stamped on the document, defaulting when absent. */
function currentSkin(): ThemeSkin {
  if (typeof document === "undefined") return DEFAULT_SKIN;
  const raw = document.documentElement.dataset.skin;
  return isThemeSkin(raw) ? raw : DEFAULT_SKIN;
}

function syncThemeColor(resolved: ResolvedTheme, skin: ThemeSkin): void {
  if (typeof document === "undefined") return;
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", themeColorFor(resolved, skin));
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function readThemePreference(storage: Pick<Storage, "getItem"> = localStorage): ThemePreference {
  let raw: string | null = null;
  try {
    raw = storage.getItem(THEME_STORAGE_KEY);
  } catch {
    return "system";
  }
  return isThemePreference(raw) ? raw : "system";
}

export function writeThemePreference(
  preference: ThemePreference,
  storage: Pick<Storage, "setItem"> = localStorage,
): void {
  try {
    storage.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // A read-only storage should never stop the theme from applying.
  }
}

export function isThemeSkin(value: unknown): value is ThemeSkin {
  return value === "graphite" || value === "vibrancy";
}

export function readThemeSkin(storage: Pick<Storage, "getItem"> = localStorage): ThemeSkin {
  let raw: string | null = null;
  try {
    raw = storage.getItem(SKIN_STORAGE_KEY);
  } catch {
    return DEFAULT_SKIN;
  }
  return isThemeSkin(raw) ? raw : DEFAULT_SKIN;
}

export function writeThemeSkin(
  skin: ThemeSkin,
  storage: Pick<Storage, "setItem"> = localStorage,
): void {
  try {
    storage.setItem(SKIN_STORAGE_KEY, skin);
  } catch {
    // A read-only storage should never stop the skin from applying.
  }
}

/** Stamps the chosen skin on the document so CSS can key off `data-skin`. */
export function applySkin(skin: ThemeSkin): ThemeSkin {
  if (typeof document === "undefined") return skin;
  document.documentElement.dataset.skin = skin;
  // The dark ground differs by skin, so the meta colour follows the skin too.
  const resolved: ResolvedTheme = document.documentElement.dataset.theme === "dark" ? "dark" : "light";
  syncThemeColor(resolved, skin);
  return skin;
}

export function systemPrefersDark(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : false;
}

export function resolveTheme(preference: ThemePreference, prefersDark = systemPrefersDark()): ResolvedTheme {
  if (preference === "system") return prefersDark ? "dark" : "light";
  return preference;
}

/** Applies the resolved theme to the document and returns what it resolved to. */
export function applyTheme(preference: ThemePreference, prefersDark = systemPrefersDark()): ResolvedTheme {
  const resolved = resolveTheme(preference, prefersDark);
  if (typeof document === "undefined") return resolved;

  document.documentElement.classList.toggle("dark", resolved === "dark");
  document.documentElement.dataset.theme = resolved;
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    document.documentElement.dataset.tauri = "";
  }

  syncThemeColor(resolved, currentSkin());

  return resolved;
}

/**
 * Re-applies the theme whenever macOS appearance changes. The listener stays
 * attached for every preference so pinning light and then returning to system
 * picks up the current appearance without a reload.
 */
export function watchSystemTheme(onChange: (prefersDark: boolean) => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => {};
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", handler);
  return () => query.removeEventListener("change", handler);
}

/** Broadcast so every mounted consumer stays in sync with a single source. */
const THEME_EVENT = "bridge:theme";

/**
 * Reads the stored preference, keeps the document in sync with it, and follows
 * macOS appearance while the preference is "system".
 */
export function useThemePreference(): {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (next: ThemePreference) => void;
  skin: ThemeSkin;
  setSkin: (next: ThemeSkin) => void;
} {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => readThemePreference());
  const [resolved, setResolved] = useState<ResolvedTheme>(() => resolveTheme(preference));
  const [skin, setSkinState] = useState<ThemeSkin>(() => readThemeSkin());

  useEffect(() => {
    setResolved(applyTheme(preference));
    return watchSystemTheme(prefersDark => setResolved(applyTheme(preference, prefersDark)));
  }, [preference]);

  useEffect(() => {
    applySkin(skin);
  }, [skin]);

  useEffect(() => {
    const onExternalChange = () => {
      setPreferenceState(readThemePreference());
      setSkinState(readThemeSkin());
    };
    window.addEventListener(THEME_EVENT, onExternalChange);
    return () => window.removeEventListener(THEME_EVENT, onExternalChange);
  }, []);

  const setPreference = useCallback((next: ThemePreference) => {
    writeThemePreference(next);
    setPreferenceState(next);
    setResolved(applyTheme(next));
    window.dispatchEvent(new Event(THEME_EVENT));
  }, []);

  const setSkin = useCallback((next: ThemeSkin) => {
    writeThemeSkin(next);
    setSkinState(next);
    applySkin(next);
    window.dispatchEvent(new Event(THEME_EVENT));
  }, []);

  return { preference, resolved, setPreference, skin, setSkin };
}
