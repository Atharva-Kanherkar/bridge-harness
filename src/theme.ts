// Theme preference: the app follows macOS appearance unless the user pins a
// mode. Both modes are first-class renderings of the same token names, so the
// only thing that changes at runtime is the `dark` class on <html>.

import { useCallback, useEffect, useState } from "react";

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "bridge.theme";

/** Window background per mode, kept in sync with `--background` in index.css. */
const THEME_COLOR: Record<ResolvedTheme, string> = {
  light: "#fafaf9",
  dark: "#000000",
};

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

  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", THEME_COLOR[resolved]);

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
} {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => readThemePreference());
  const [resolved, setResolved] = useState<ResolvedTheme>(() => resolveTheme(preference));

  useEffect(() => {
    setResolved(applyTheme(preference));
    return watchSystemTheme(prefersDark => setResolved(applyTheme(preference, prefersDark)));
  }, [preference]);

  useEffect(() => {
    const onExternalChange = () => setPreferenceState(readThemePreference());
    window.addEventListener(THEME_EVENT, onExternalChange);
    return () => window.removeEventListener(THEME_EVENT, onExternalChange);
  }, []);

  const setPreference = useCallback((next: ThemePreference) => {
    writeThemePreference(next);
    setPreferenceState(next);
    setResolved(applyTheme(next));
    window.dispatchEvent(new Event(THEME_EVENT));
  }, []);

  return { preference, resolved, setPreference };
}
