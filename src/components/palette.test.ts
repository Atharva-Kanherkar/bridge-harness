import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { SYNTAX_CLASSES } from "./highlight";
import { EDITOR_SYNTAX_CLASSES } from "./editor/highlighter";

/**
 * The palette's own tests.
 *
 * Bridge paints code through three renderers — Shiki's scope classifier for
 * chat and diffs, a Lezer `tagHighlighter` for the editor, and the diff
 * viewer's row tints — and the only thing keeping them in agreement is that
 * they emit the same class names against one set of `--syn-*` tokens. That
 * agreement used to be a convention enforced by review, and review missed it:
 * `index.css` carried `.tok-function`, `.tok-tagName`, `.tok-attributeName`
 * and a dozen more rules that no renderer could ever emit. These tests parse
 * the stylesheet so the same thing cannot happen again.
 */
const CSS = readFileSync(new URL("../index.css", import.meta.url), "utf8");
/** Comments mention class and token names in prose; strip them before any
 *  structural claim, or a doc comment can satisfy — or break — an assertion. */
const RULES = CSS.replace(/\/\*[\s\S]*?\*\//g, "");

/** The `--syn-*` names declared in each CSS block, keyed by that block's
 *  opening brace, so a comment between two declarations cannot split a
 *  theme block into two apparent blocks. */
function synTokensPerBlock(): string[][] {
  const blocks = new Map<number, string[]>();
  const stack: number[] = [];
  let offset = 0;
  for (const line of RULES.split("\n")) {
    const declaration = /^\s*(--syn-[a-z-]+)\s*:/.exec(line);
    if (declaration) {
      const owner = stack[stack.length - 1] ?? -1;
      const group = blocks.get(owner) ?? [];
      group.push(declaration[1]);
      blocks.set(owner, group);
    }
    for (const character of line) {
      if (character === "{") stack.push(offset);
      else if (character === "}") stack.pop();
      offset += 1;
    }
    offset += 1;
  }
  return [...blocks.values()];
}

const EXPECTED_TOKENS = [
  "--syn-comment", "--syn-keyword", "--syn-string", "--syn-number",
  "--syn-function", "--syn-type", "--syn-tag", "--syn-punct",
  "--syn-operator", "--syn-variable", "--syn-property", "--syn-param",
  "--syn-regex", "--syn-addition", "--syn-deletion",
];

/** WCAG 2.1 relative luminance and contrast ratio. */
function luminance(hex: string): number {
  const channel = (offset: number) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

/**
 * Every `--name: #rrggbb` declared in each CSS block, in file order.
 *
 * Deliberately *not* restricted to `--syn-*`: the emitted classes are not all
 * painted from the syntax ramp — `.stx-meta` takes `--color-muted-foreground`,
 * `.stx-invalid` and `.stx-deletion` take `--color-destructive`,
 * `.stx-addition` takes `--color-success` — and a contrast assertion that only
 * reads the ramp is narrower than the claim `index.css` makes above it.
 */
function hexDeclarationsPerBlock(): Record<string, string>[] {
  const blocks: Record<string, string>[] = [];
  const stack: number[] = [];
  const owners = new Map<number, Record<string, string>>();
  let offset = 0;
  for (const line of RULES.split("\n")) {
    const declaration = /^\s*(--[a-z-]+)\s*:\s*(#[0-9a-fA-F]{3,8})\s*;/.exec(line);
    if (declaration) {
      const owner = stack[stack.length - 1] ?? -1;
      if (!owners.has(owner)) { const group: Record<string, string> = {}; owners.set(owner, group); blocks.push(group); }
      owners.get(owner)![declaration[1]] = declaration[2];
    }
    for (const character of line) {
      if (character === "{") stack.push(offset);
      else if (character === "}") stack.pop();
      offset += 1;
    }
    offset += 1;
  }
  return blocks;
}

/** The two theme blocks, identified by carrying the ramp rather than by
 *  position, so inserting a block above `:root` cannot silently reindex them. */
function themeBlocks(): [light: Record<string, string>, dark: Record<string, string>] {
  const blocks = hexDeclarationsPerBlock().filter(block => "--syn-comment" in block);
  expect(blocks, "expected exactly two blocks declaring the syntax ramp").toHaveLength(2);
  return [blocks[0], blocks[1]];
}

/** `@theme`'s `--color-x: var(--x)` aliases, so a rule written against a
 *  Tailwind token resolves to the same value the browser would use. */
const ALIASES: Record<string, string> = Object.fromEntries(
  [...RULES.matchAll(/^\s*(--color-[a-z-]+)\s*:\s*var\((--[a-z-]+)\)/gm)].map(match => [match[1], match[2]]),
);

/**
 * A CSS colour expression → `#rrggbb`, or `null` when the expression is not a
 * flat colour this test can reason about.
 *
 * Returning `null` rather than skipping is the point: the caller turns an
 * unresolvable value into a *failure*. The assertion this replaces matched
 * only six-digit lowercase hex, so writing a ramp entry as `#ABC`, `rgb(...)`
 * or `light-dark(...)` would have dropped it from the contrast check with no
 * test going red — a claim quietly narrowing itself is exactly the failure
 * `palette.test.ts` exists to prevent.
 */
function resolveColor(expression: string, theme: Record<string, string>, depth = 0): string | null {
  const value = expression.trim();
  if (depth > 4) return null;
  const hex = /^#([0-9a-fA-F]{6})$/.exec(value);
  if (hex) return `#${hex[1].toLowerCase()}`;
  const reference = /^var\((--[a-z-]+)\)$/.exec(value);
  if (!reference) return null;
  const name = reference[1];
  if (theme[name]) return resolveColor(theme[name], theme, depth + 1);
  if (ALIASES[name]) return resolveColor(`var(${ALIASES[name]})`, theme, depth + 1);
  return null;
}

/** `color-mix(in srgb, <colour> N%, transparent)` laid over an opaque one. */
function composite(background: string, tint: string | null, theme: Record<string, string>): string | null {
  if (!tint) return background;
  const mix = /^color-mix\(in srgb,\s*(.+?)\s+(\d+)%,\s*transparent\)$/.exec(tint.trim());
  if (!mix) return null;
  const over = resolveColor(mix[1], theme);
  if (!over) return null;
  const alpha = Number(mix[2]) / 100;
  const channel = (offset: number) => {
    const front = parseInt(over.slice(offset, offset + 2), 16);
    const back = parseInt(background.slice(offset, offset + 2), 16);
    return Math.round(front * alpha + back * (1 - alpha)).toString(16).padStart(2, "0");
  };
  return `#${channel(1)}${channel(3)}${channel(5)}`;
}

/** Each `.stx-*` rule's own `color` and `background`, unresolved. */
function syntaxRules(): Map<string, { color?: string; background?: string }> {
  const rules = new Map<string, { color?: string; background?: string }>();
  for (const match of RULES.matchAll(/^\.(stx(?:-[a-z]+)?)\s*\{([^}]*)\}/gm)) {
    const body = match[2];
    const color = /(?:^|;)\s*color\s*:\s*([^;]+)/.exec(body)?.[1];
    const background = /(?:^|;)\s*background\s*:\s*([^;]+)/.exec(body)?.[1];
    rules.set(match[1], { color, background });
  }
  return rules;
}

/**
 * The `--code` surfaces syntax is painted on. Listed explicitly, and checked
 * against the stylesheet below, so changing a code background fails this test
 * rather than silently invalidating the contrast assertion. The third is the
 * native vibrancy skin, which overrides `--code` but *not* the syntax ramp —
 * so the dark colours have to clear contrast against the lighter of the two.
 */
const CODE_BACKGROUNDS = { light: ["#f3f3f1"], dark: ["#0a0a0a", "#0e0e0d"] };

describe("the syntax palette", () => {
  it("still paints onto exactly the code backgrounds this file knows about", () => {
    const declared = [...RULES.matchAll(/^\s*--code:\s*(#[0-9a-f]{6})\s*;/gm)].map(match => match[1]);
    expect([...declared].sort()).toEqual([...CODE_BACKGROUNDS.light, ...CODE_BACKGROUNDS.dark].sort());
  });

  it("clears WCAG 4.5:1 for every emitted class, on every code background", () => {
    // The claim this replaces was a comment asserting 4.5:1 that was wrong for
    // two greys (light comment 3.74, dark comment 3.90). A design claim worth
    // writing down is worth asserting — and asserting for what is actually
    // *painted*, which is the class, not the token. Nine of the buckets take
    // their colour from somewhere other than the ramp: `.stx-meta`,
    // `.stx-invalid`, `.stx-addition` and `.stx-deletion` from chrome tokens,
    // and `.stx-emphasis`/`.stx-strong`/`.stx-strike` — plus `.stx` itself —
    // from inherited `--code-foreground`. Reading `--syn-*` declarations
    // checked none of them.
    const [light, dark] = themeBlocks();
    const rules = syntaxRules();
    const inherited = rules.get("stx")?.color;
    expect(inherited, "`.stx` must set the colour the unpainted buckets inherit").toBeTruthy();
    const failures: string[] = [];
    for (const [theme, backgrounds, label] of [[light, CODE_BACKGROUNDS.light, "light"], [dark, CODE_BACKGROUNDS.dark, "dark"]] as const) {
      for (const name of SYNTAX_CLASSES) {
        const rule = rules.get(name);
        // `gives every renderer-emitted class a rule` covers absence; here a
        // present rule that cannot be resolved is the failure worth naming.
        if (!rule) continue;
        const foreground = resolveColor(rule.color ?? inherited!, theme);
        if (!foreground) { failures.push(`${label} .${name}: cannot resolve colour ${rule.color ?? inherited}`); continue; }
        for (const background of backgrounds) {
          const surface = composite(background, rule.background ?? null, theme);
          if (!surface) { failures.push(`${label} .${name}: cannot resolve background ${rule.background}`); continue; }
          const ratio = contrast(foreground, surface);
          if (ratio < 4.5) failures.push(`${label} .${name} ${foreground} on ${surface} = ${ratio.toFixed(2)}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });

  it("leaves no ramp token out of the contrast check", () => {
    // The guard on the guard. `themeBlocks` reads flat hex, so a token
    // rewritten as `rgb(...)` or `light-dark(...)` would vanish from the block
    // instead of failing — the silent-skip shape the old six-digit-lowercase
    // regex had. Every expected token must resolve, in both themes.
    for (const theme of themeBlocks()) {
      for (const token of EXPECTED_TOKENS) {
        expect(resolveColor(`var(${token})`, theme), `${token} does not resolve to a flat colour`).toMatch(/^#[0-9a-f]{6}$/);
      }
    }
  });

  it("declares every token in both theme blocks", () => {
    const blocks = synTokensPerBlock();
    // Light and dark. A third block would be a mistake worth failing on.
    expect(blocks).toHaveLength(2);
    for (const block of blocks) {
      expect([...block].sort()).toEqual([...EXPECTED_TOKENS].sort());
    }
  });

  it("exposes the same ramp as Tailwind colour tokens", () => {
    for (const token of EXPECTED_TOKENS) {
      const utility = token.replace("--syn-", "--color-syn-");
      expect(RULES, `${utility} missing from @theme`).toContain(`${utility}: var(${token})`);
    }
  });

  it("gives every renderer-emitted class a rule", () => {
    for (const name of SYNTAX_CLASSES) {
      expect(RULES, `.${name} has no rule in index.css`).toMatch(new RegExp(`^\\.${name}\\s*[,{]`, "m"));
    }
  });

  it("keeps the editor highlighter inside the shared vocabulary", () => {
    // The failure this prevents: adding a bucket to `editor/highlighter.ts`,
    // shipping it, and having the editor render that token as bare
    // `--code-foreground` because nothing declared its colour.
    for (const name of EDITOR_SYNTAX_CLASSES) {
      expect(SYNTAX_CLASSES, `${name} is not in SYNTAX_CLASSES`).toContain(name);
    }
  });

  it("carries no leftover .tok-* rules", () => {
    // `@lezer/highlight`'s stock `classHighlighter` is gone; any `.tok-*`
    // rule left behind is by definition dead, because nothing emits it.
    expect(RULES.match(/^\.tok-[a-zA-Z-]+/gm) ?? []).toEqual([]);
  });

  it("keeps chrome achromatic — no chrome token takes a syntax hue", () => {
    // Graphite & Paper: colour belongs to code, not to the app around it. The
    // `--color-syn-*` family is the one deliberate exception, because those
    // exist precisely to hand a syntax hue to a syntax-coloured element.
    const leaks = [...RULES.matchAll(/^\s*(--color-[a-z-]+)\s*:\s*var\((--syn-[a-z-]+)\)/gm)]
      .map(match => match[1])
      .filter(name => !name.startsWith("--color-syn-"));
    expect(leaks).toEqual([]);
  });
});
