// Omarchy palette -> Bridge tokens. The one place the mapping lives.
//
// An Omarchy theme is a flat `colors.toml` (background, foreground, accent,
// the ANSI-ish hues, a `mode`). Bridge already renders every surface from its
// own token names, so the skin is a table from one vocabulary to the other: no
// new token names, no palette literals in source. Harness and chart tints are
// deliberately absent: they identify vendors, not the theme.
//
// `landing/app/lib/omarchyTheme.ts` is a byte-identical copy so the landing
// page's Omarchy mode retints with exactly this table. A landing test fails if
// the two drift; edit here, then copy.

export type OmarchyMode = "dark" | "light";

/** A theme as read from `colors.toml`: its mode plus whatever hex colours it named. */
export type OmarchyPalette = {
  mode: OmarchyMode;
  colors: Record<string, string>;
};

const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && HEX.test(value);
}

/* A role reads the first key the theme defines. Omarchy themes are not
   uniform: `white`, `solitude` and `last-horizon` ship without `orange`. */
const ROLE_FALLBACKS: Record<string, string[]> = {
  background: ["background"],
  foreground: ["foreground"],
  accent: ["accent", "blue"],
  bright_foreground: ["bright_foreground", "light_foreground", "foreground"],
  dark_foreground: ["dark_foreground", "muted", "foreground"],
  dark_background: ["dark_background", "background"],
  darker_background: ["darker_background", "dark_background", "background"],
  lighter_background: ["lighter_background", "selection", "background"],
  selection: ["selection", "lighter_background", "background"],
  muted: ["muted", "dark_foreground", "foreground"],
  red: ["red", "bright_red", "accent"],
  green: ["green", "bright_green", "accent"],
  yellow: ["yellow", "bright_yellow", "accent"],
  orange: ["orange", "bright_yellow", "yellow", "accent"],
  blue: ["blue", "bright_blue", "accent"],
  magenta: ["magenta", "bright_magenta", "accent"],
  cyan: ["cyan", "bright_cyan", "accent"],
  bright_cyan: ["bright_cyan", "cyan", "accent"],
};

type Role = keyof typeof ROLE_FALLBACKS;

const mix = (base: string, over: string, pct: number) => `color-mix(in srgb, ${over} ${pct}%, ${base})`;
const tint = (color: string, pct: number) => `color-mix(in srgb, ${color} ${pct}%, transparent)`;

/** The table. Each entry reads palette roles and returns a CSS value. */
const TOKENS: Record<string, (c: (role: Role) => string, light: boolean) => string> = {
  // Surfaces
  "--sidebar": c => c("dark_background"),
  "--background": c => c("background"),
  "--card": c => mix(c("background"), c("lighter_background"), 55),
  "--popover": c => c("lighter_background"),
  "--muted": c => c("lighter_background"),
  "--secondary": c => c("lighter_background"),
  "--code": c => c("darker_background"),
  // Ink
  "--foreground": c => c("bright_foreground"),
  "--card-foreground": c => c("bright_foreground"),
  "--popover-foreground": c => c("bright_foreground"),
  "--secondary-foreground": c => c("bright_foreground"),
  "--body": c => c("foreground"),
  "--muted-foreground": c => mix(c("foreground"), c("dark_foreground"), 55),
  "--faint": c => c("dark_foreground"),
  "--faint-2": c => c("muted"),
  "--selection": c => c("selection"),
  "--selection-foreground": c => c("bright_foreground"),
  "--code-foreground": c => c("foreground"),
  "--code-highlight": c => tint(c("foreground"), 7),
  "--accent": c => tint(c("foreground"), 8),
  "--accent-foreground": c => c("bright_foreground"),
  // Lines
  "--border": (c, light) => mix(c("background"), c("foreground"), light ? 12 : 9),
  "--border-card": (c, light) => mix(c("background"), c("foreground"), light ? 16 : 13),
  "--input": (c, light) => mix(c("background"), c("foreground"), light ? 16 : 13),
  // Signal
  "--primary": c => c("accent"),
  "--primary-foreground": c => c("darker_background"),
  "--ring": c => c("accent"),
  "--success": c => c("green"),
  "--warning": c => c("yellow"),
  "--info": c => c("blue"),
  "--destructive": c => c("red"),
  // Syntax ramp
  "--syn-comment": c => c("dark_foreground"),
  "--syn-keyword": c => c("magenta"),
  "--syn-string": c => c("green"),
  "--syn-number": c => c("orange"),
  "--syn-function": c => c("blue"),
  "--syn-type": c => c("yellow"),
  "--syn-tag": c => c("cyan"),
  "--syn-punct": c => c("dark_foreground"),
  "--syn-operator": c => c("bright_cyan"),
  "--syn-variable": c => c("bright_foreground"),
  "--syn-property": c => c("cyan"),
  "--syn-param": c => c("orange"),
  "--syn-regex": c => c("bright_cyan"),
  "--syn-addition": c => c("green"),
  "--syn-deletion": c => c("red"),
  // Sidebar
  "--sidebar-foreground": c => mix(c("foreground"), c("dark_foreground"), 45),
  "--sidebar-accent": c => tint(c("foreground"), 9),
  "--sidebar-accent-foreground": c => c("bright_foreground"),
  "--sidebar-border": c => mix(c("dark_background"), c("foreground"), 8),
  "--sidebar-primary": c => c("accent"),
  "--sidebar-primary-foreground": c => c("darker_background"),
  "--sidebar-ring": c => c("accent"),
  // Shape: Hyprland draws the window frame, so the app stays square.
  "--window-radius": () => "0px",
  "--radius": () => "0.3rem",
};

/** Every custom property the skin can set, so leaving it can remove them all. */
export const OMARCHY_TOKEN_NAMES: readonly string[] = Object.keys(TOKENS);

/**
 * Bridge tokens for a palette, or an empty map when the theme lacks the three
 * colours everything else derives from.
 */
export function omarchyTokens(palette: OmarchyPalette): Record<string, string> {
  const colors: Record<string, string> = {};
  for (const [key, value] of Object.entries(palette.colors)) {
    if (isHexColor(value)) colors[key] = value;
  }
  if (!colors.background || !colors.foreground || !(colors.accent ?? colors.blue)) return {};
  const role = (name: Role) => {
    for (const key of ROLE_FALLBACKS[name]) {
      if (colors[key]) return colors[key];
    }
    return colors.foreground;
  };
  const light = palette.mode === "light";
  const out: Record<string, string> = {};
  for (const [name, value] of Object.entries(TOKENS)) out[name] = value(role, light);
  return out;
}
