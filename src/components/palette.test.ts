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
  "--syn-regex",
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

/** `--syn-*` name → value for each theme block, in file order (light, dark). */
function synValuesPerBlock(): Record<string, string>[] {
  const blocks: Record<string, string>[] = [];
  const stack: number[] = [];
  const owners = new Map<number, Record<string, string>>();
  let offset = 0;
  for (const line of RULES.split("\n")) {
    const declaration = /^\s*(--syn-[a-z-]+)\s*:\s*(#[0-9a-f]{6})\s*;/.exec(line);
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

  it("clears WCAG 4.5:1 on every code background", () => {
    // The claim this replaces was a comment asserting 4.5:1 that was wrong for
    // two greys (light comment 3.74, dark comment 3.90). A design claim worth
    // writing down is worth asserting.
    const [light, dark] = synValuesPerBlock();
    const failures: string[] = [];
    for (const [block, backgrounds] of [[light, CODE_BACKGROUNDS.light], [dark, CODE_BACKGROUNDS.dark]] as const) {
      for (const [token, value] of Object.entries(block)) {
        for (const background of backgrounds) {
          const ratio = contrast(value, background);
          if (ratio < 4.5) failures.push(`${token} ${value} on ${background} = ${ratio.toFixed(2)}`);
        }
      }
    }
    expect(failures).toEqual([]);
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
