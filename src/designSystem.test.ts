import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

// "Graphite & Paper" guard rails.
//
// The chrome is achromatic and every colour resolves through a semantic token,
// so both light and dark are renderings of the same names. That property is
// easy to break one className at a time, so it is asserted here rather than
// left to review.

const SRC = join(__dirname);

/** Files allowed to name a raw colour, each for a stated reason. */
const COLOR_LITERAL_ALLOWLIST = new Set([
  // Mirrors --background per mode for the <meta name="theme-color"> tag, which
  // cannot read a CSS custom property.
  "theme.ts",
]);

const BANNED_PALETTES = [
  "neutral", "gray", "zinc", "slate", "stone",
  "emerald", "green", "lime", "teal", "cyan", "sky", "blue", "indigo",
  "violet", "purple", "fuchsia", "pink", "rose", "red", "orange", "amber", "yellow",
];
const UTILITY_PREFIXES = ["text", "bg", "border", "ring", "divide", "fill", "stroke", "from", "via", "to", "shadow", "outline", "accent", "caret", "decoration"];

const paletteClassPattern = new RegExp(
  `\\b(?:${UTILITY_PREFIXES.join("|")})-(?:${BANNED_PALETTES.join("|")})-\\d{2,3}\\b`,
  "g",
);
const whiteAlphaPattern = /\b(?:text|bg|border|ring|divide|from|via|to)-(?:white|black)\/(?:\[[\d.]+\]|\d{1,3})/g;
const hexLiteralPattern = /#[0-9a-fA-F]{6}\b/g;

/** Class strings that put a blur on a surface which is supposed to sit at rest. */
const RESTING_SURFACE = /\b(?:bg-card|bg-sidebar|u-surface|u-glass-soft)\b/;
function blurredRestingSurfaces(text: string): string[] {
  // Blur is reserved for genuinely floating layers (scrims, dialogs, popovers),
  // so the checkable rule is that no single class string both names a resting
  // surface and blurs what is behind it.
  const classStrings = text.match(/"[^"\n]*"|'[^'\n]*'|`[^`\n]*`/g) ?? [];
  return classStrings.filter(value => value.includes("backdrop-blur") && RESTING_SURFACE.test(value));
}

function sourceFiles(dir: string, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      sourceFiles(full, acc);
      continue;
    }
    if (![".ts", ".tsx"].includes(extname(entry.name))) continue;
    if (entry.name.includes(".test.")) continue;
    acc.push(full);
  }
  return acc;
}

const files = sourceFiles(SRC).map(path => ({
  path,
  name: relative(SRC, path),
  text: readFileSync(path, "utf8"),
}));

const offenders = (pattern: RegExp, skip: (name: string) => boolean = () => false) =>
  files
    .filter(file => !skip(file.name))
    .flatMap(file => {
      const hits = file.text.match(new RegExp(pattern.source, pattern.flags)) ?? [];
      return hits.length ? [`${file.name}: ${[...new Set(hits)].join(", ")}`] : [];
    });

describe("design system", () => {
  it("has source files to check", () => {
    expect(files.length).toBeGreaterThan(20);
  });

  it("routes every colour through a semantic token, never a palette class", () => {
    // Use text-foreground / text-muted-foreground / bg-card / border-border and
    // the status tokens text-success | warning | info | destructive instead.
    expect(offenders(paletteClassPattern)).toEqual([]);
  });

  it("uses border and surface tokens instead of white/black alpha washes", () => {
    // bg-white/[0.05] only exists in a dark-only world; bg-accent and
    // border-border resolve correctly in both modes.
    expect(offenders(whiteAlphaPattern)).toEqual([]);
  });

  it("keeps raw colour literals out of components", () => {
    expect(offenders(hexLiteralPattern, name => COLOR_LITERAL_ALLOWLIST.has(name))).toEqual([]);
  });

  it("only uses on-solid status ink on a solid status fill", () => {
    // --success-foreground and friends are the ink for a full-strength
    // bg-success; on a wash or a neutral surface they are near-black in dark
    // mode. The hue itself (text-success) is the ink everywhere else.
    const statuses = ["success", "warning", "info", "destructive"];
    const flagged = files.flatMap(file => {
      const classStrings = file.text.match(/"[^"\n]*"|'[^'\n]*'|`[^`\n]*`/g) ?? [];
      return classStrings
        .filter(value =>
          statuses.some(status => {
            if (!value.includes(`text-${status}-foreground`)) return false;
            const solid = new RegExp(`\\bbg-${status}(?![\\w/-])`).test(value);
            return !solid;
          }),
        )
        .map(value => `${file.name}: ${value.slice(0, 120)}`);
    });
    expect(flagged).toEqual([]);
  });

  it("keeps blur off resting surfaces", () => {
    // Elevation is a lightness ladder; only genuinely floating layers may blur.
    const flagged = files.flatMap(file => {
      const hits = blurredRestingSurfaces(file.text);
      return hits.length ? [`${file.name}: ${hits.join(" | ")}`] : [];
    });
    expect(flagged).toEqual([]);
  });
});

describe("theme tokens", () => {
  const css = readFileSync(join(SRC, "index.css"), "utf8");

  const tokenValue = (block: string, token: string) => {
    const scope = css.slice(css.indexOf(block));
    const match = scope.match(new RegExp(`--${token}:\\s*([^;]+);`));
    return match?.[1].trim();
  };

  it("defines the full ladder in both modes", () => {
    for (const token of ["sidebar", "background", "card", "popover", "border", "foreground", "muted-foreground", "primary", "ring"]) {
      expect(tokenValue(":root {", token), `light --${token}`).toBeTruthy();
      expect(tokenValue(".dark {", token), `dark --${token}`).toBeTruthy();
    }
  });

  it("locks the graphite and paper grounds", () => {
    expect(tokenValue(":root {", "background")).toBe("#fafaf9");
    expect(tokenValue(".dark {", "background")).toBe("#212120");
  });

  it("keeps the ladder rungs distinct within each mode", () => {
    for (const block of [":root {", ".dark {"]) {
      const rungs = ["sidebar", "background", "card", "popover"].map(token => tokenValue(block, token));
      // popover may equal card in paper (both pure white), but the canvas and
      // the raised tier must never collapse into each other.
      expect(new Set(rungs).size, `${block} ladder`).toBeGreaterThanOrEqual(3);
      expect(tokenValue(block, "background")).not.toBe(tokenValue(block, "card"));
      expect(tokenValue(block, "muted")).not.toBe(tokenValue(block, "card"));
    }
  });

  it("keeps primary an inversion rather than a hue", () => {
    expect(tokenValue(":root {", "primary")).toBe(tokenValue(":root {", "foreground"));
    expect(tokenValue(".dark {", "primary")).toBe(tokenValue(".dark {", "foreground"));
  });

  it("does not hardcode a colour scheme on the document", () => {
    // A pinned `color-scheme: dark` on html is what made light mode unreachable.
    expect(css).toMatch(/:root\s*\{[^}]*color-scheme:\s*light/);
    expect(css).toMatch(/\.dark\s*\{[^}]*color-scheme:\s*dark/);
  });

  it("retires the starfield", () => {
    expect(css).not.toContain("space-drift");
    expect(css).not.toContain("star-twinkle");
    expect(css).not.toContain(".space-dark");
  });

  it("honours reduced motion", () => {
    expect(css).toContain("prefers-reduced-motion");
  });
});
