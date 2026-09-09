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

describe("the syntax palette", () => {
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
