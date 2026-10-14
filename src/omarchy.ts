// The Omarchy theme as the desktop shell reports it.
//
// On Linux with Omarchy, `src-tauri/src/omarchy.rs` sends the active theme's
// palette, mode and font, and sends it again on every theme switch. This
// module holds the latest payload and turns it into custom properties on
// <html>; `theme.ts` decides when the `omarchy` skin is the one showing. On
// macOS, in mock mode, and on Linux without Omarchy nothing ever arrives, so
// the skin stays hidden and Graphite renders as it always has.

import { useEffect, useState } from "react";
import { isHexColor, OMARCHY_TOKEN_NAMES, omarchyTokens, type OmarchyMode } from "./lib/omarchyTheme";

/** Mirrors `ThemePayload` in `src-tauri/src/omarchy.rs`. */
export type OmarchyThemePayload = {
  available: boolean;
  themeName: string | null;
  mode: OmarchyMode;
  colors: Record<string, string>;
  font: string | null;
};

/** Shell to webview, and the request the webview sends once it listens. */
export const OMARCHY_THEME_EVENT = "bridge-omarchy-theme";
export const OMARCHY_THEME_REQUEST_EVENT = "bridge-omarchy-theme-request";

/** Overrides the app's sans, mono and display stacks (see `index.css`). */
export const SKIN_FONT_PROPERTY = "--skin-font";

const CHANGE_EVENT = "bridge:omarchy-theme";

let current: OmarchyThemePayload | null = null;

/** The shell's payload is still input: keep only what the skin can use. */
export function parseOmarchyPayload(raw: unknown): OmarchyThemePayload | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  if (value.available !== true || !value.colors || typeof value.colors !== "object") return null;
  const colors: Record<string, string> = {};
  for (const [key, color] of Object.entries(value.colors as Record<string, unknown>)) {
    if (isHexColor(color)) colors[key] = color;
  }
  return {
    available: true,
    themeName: typeof value.themeName === "string" ? value.themeName : null,
    mode: value.mode === "light" ? "light" : "dark",
    colors,
    font: typeof value.font === "string" && value.font.trim() ? value.font.trim() : null,
  };
}

export function currentOmarchyTheme(): OmarchyThemePayload | null {
  return current;
}

/** Accept a payload from the shell (or a test) and tell every subscriber. */
export function receiveOmarchyTheme(raw: unknown): void {
  const payload = parseOmarchyPayload(raw);
  if (!payload) return;
  current = payload;
  if (typeof window !== "undefined") window.dispatchEvent(new Event(CHANGE_EVENT));
}

/** Tests only: forget the payload, as if Omarchy were never detected. */
export function resetOmarchyTheme(): void {
  current = null;
}

export function onOmarchyTheme(handler: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, handler);
  return () => window.removeEventListener(CHANGE_EVENT, handler);
}

/** A CSS font stack from a family name, quoted so any name stays one family. */
export function skinFontStack(family: string | null): string {
  const quoted = family ? `"${family.replace(/["\\]/g, "")}", ` : "";
  return `${quoted}ui-monospace, monospace`;
}

/**
 * Paint the payload onto <html>. Returns false (and paints nothing) when the
 * palette is unusable, so the caller can keep rendering Graphite.
 */
export function paintOmarchyTheme(payload: OmarchyThemePayload, root: HTMLElement = document.documentElement): boolean {
  const tokens = omarchyTokens(payload);
  if (!Object.keys(tokens).length) return false;
  for (const [name, value] of Object.entries(tokens)) root.style.setProperty(name, value);
  // Omarchy is monospace everywhere; the skin follows it.
  root.style.setProperty(SKIN_FONT_PROPERTY, skinFontStack(payload.font));
  return true;
}

/** Remove everything `paintOmarchyTheme` set. */
export function clearOmarchyTheme(root: HTMLElement = document.documentElement): void {
  for (const name of OMARCHY_TOKEN_NAMES) root.style.removeProperty(name);
  root.style.removeProperty(SKIN_FONT_PROPERTY);
}

/** Listen for the shell's payload and ask for the current one. Tauri only. */
export function startOmarchyThemeSync(): void {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  void import("@tauri-apps/api/event").then(async ({ emit, listen }) => {
    await listen(OMARCHY_THEME_EVENT, event => receiveOmarchyTheme(event.payload));
    await emit(OMARCHY_THEME_REQUEST_EVENT);
  }).catch(() => {
    // No theme sync is the same as no Omarchy: the app renders Graphite.
  });
}

/** The latest payload, re-rendering when the theme switches. */
export function useOmarchyTheme(): OmarchyThemePayload | null {
  const [payload, setPayload] = useState(currentOmarchyTheme);
  useEffect(() => {
    setPayload(currentOmarchyTheme());
    return onOmarchyTheme(() => setPayload(currentOmarchyTheme()));
  }, []);
  return payload;
}
