// Keyboard shortcuts as each platform writes them.
//
// Bridge's chords say `meta` for the primary modifier: Command on macOS,
// Control everywhere else (see `keymap.ts`). This is the one place that turns
// a chord into text, so no component spells a modifier itself. macOS prints
// glyphs in its fixed order (⌃⌥⇧⌘K); Linux and Windows print words joined by
// `+` (Ctrl+Alt+Shift+K), the way GNOME, KDE and Omarchy's own menus do.

export type ShortcutModifiers = { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean };

/** The macOS modifier glyphs, for the few places that print one on its own. */
export const MAC_GLYPHS = { ctrl: "⌃", alt: "⌥", shift: "⇧", meta: "⌘" } as const;

type NavigatorLike = { platform?: string; userAgentData?: { platform?: string } };

/**
 * True unless the platform positively says Linux or Windows. Bridge ships on
 * macOS first, and an unknown platform (jsdom, an empty string) keeps the
 * macOS rendering every existing surface was designed around.
 */
export function isMacPlatform(nav: NavigatorLike | undefined = typeof navigator === "undefined" ? undefined : navigator): boolean {
  const platform = `${nav?.userAgentData?.platform ?? ""} ${nav?.platform ?? ""}`.toLowerCase();
  return !/linux|win|x11|cros/.test(platform);
}

/** A chord as text: `⌥⌘F` on macOS, `Ctrl+Alt+F` elsewhere. */
export function formatShortcut(modifiers: ShortcutModifiers, key: string, mac = isMacPlatform()): string {
  if (mac) {
    const glyphs = [
      modifiers.ctrl ? MAC_GLYPHS.ctrl : "",
      modifiers.alt ? MAC_GLYPHS.alt : "",
      modifiers.shift ? MAC_GLYPHS.shift : "",
      modifiers.meta ? MAC_GLYPHS.meta : "",
    ];
    return `${glyphs.join("")}${key}`;
  }
  const words = [
    modifiers.meta || modifiers.ctrl ? "Ctrl" : "",
    modifiers.alt ? "Alt" : "",
    modifiers.shift ? "Shift" : "",
  ].filter(Boolean);
  return [...words, key].join("+");
}

/** The primary modifier plus a key, the most common case (`⌘S`, `Ctrl+S`). */
export function modKey(key: string, mac = isMacPlatform()): string {
  return formatShortcut({ meta: true }, key, mac);
}
