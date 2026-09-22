import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { highlightTree } from "@lezer/highlight";
import type { Language } from "@codemirror/language";
import { javascriptLanguage, tsxLanguage } from "@codemirror/lang-javascript";
import { markdownLanguage } from "@codemirror/lang-markdown";
import { colorizeCode, SYNTAX_CLASSES } from "./highlight";
import { bridgeHighlighter } from "./editor/highlighter";

/**
 * Emit coverage.
 *
 * The bug this whole change set started from was a *reachability* bug: 23
 * `.tok-*` rules sat in `index.css` that no renderer could ever emit, so
 * functions, JSX tags and attributes went uncoloured while the stylesheet
 * looked complete. `palette.test.ts` proves the other direction — every class
 * a renderer emits has a colour. Neither of those catches a bucket that is
 * declared, styled, and simply never produced.
 *
 * So this file runs both renderers over real fixtures and asserts that every
 * bucket in `SYNTAX_CLASSES` is actually reached by at least one of them. It
 * is the assertion that would have caught the original problem, and it caught
 * two more while being written: `stx-strike` (unreachable because Shiki merges
 * `~~s~~` into one token whose last scope is the closing delimiter) and
 * `stx-regex`'s delimiters landing in `stx-string`.
 */

/** Fixtures chosen to exercise buckets, not to be pretty. */
const SHIKI_FIXTURES: [code: string, lang: string][] = [
  ["const total = items.length;", "typescript"],
  ["const re = /a+b/g;", "typescript"],
  ["const f = (alpha: Foo) => alpha ?? bar.baz(1);", "typescript"],
  ['const s = "a\\nb";', "typescript"],
  ['<App title={x} className="y" />', "tsx"],
  ["class A { m() { return this.x; } }", "typescript"],
  ["# Title\n\n**bold**, *soft*, ~~gone~~, [link](https://example.com)\n", "markdown"],
  ["def f(a, b=2):\n  return a  # note", "python"],
  ["fn main() { let v: Vec<u8> = vec![]; }", "rust"],
  ['{"a": 1, "b": [true, null]}', "json"],
  ["body { color: #fff; }", "css"],
  ["<!-- c --><div id=\"x\">t</div>", "xml"],
  ["@@ -1 +1 @@\n-old\n+new", "diff"],
  // Deliberately malformed: `stx-invalid` is only reachable from a grammar's
  // `invalid.illegal` scope, and a bucket with no fixture is a bucket nobody
  // can prove is alive.
  ["class 1Bad:", "python"],
];

const LEZER_FIXTURES: [code: string, language: Language][] = [
  ["const f = (a) => obj.m(a) ?? /re/g;", javascriptLanguage],
  ["const el = <div className=\"x\">{y}</div>; type T = { k: number };", tsxLanguage],
  ["# Title\n\n**bold** *soft* ~~gone~~ <https://example.com>\n", markdownLanguage],
];

function shikiClasses(html: string): string[] {
  return [...html.matchAll(/class="(stx-[a-z]+)"/g)].map(match => match[1]);
}

function lezerClasses(code: string, language: Language): string[] {
  const emitted: string[] = [];
  highlightTree(language.parser.parse(code), bridgeHighlighter, (_from, _to, classes) => {
    emitted.push(...classes.split(" ").filter(Boolean));
  });
  return emitted;
}

describe("syntax bucket coverage", () => {
  // Assert real grammar coverage independently of Shiki's wall-clock budget;
  // highlight.test.ts exercises the budget with a deliberately advancing clock.
  beforeEach(() => { vi.spyOn(Date, "now").mockReturnValue(0); });
  afterEach(() => { vi.restoreAllMocks(); });

  it("emits nothing outside the shared vocabulary", async () => {
    for (const [code, lang] of SHIKI_FIXTURES) {
      for (const name of shikiClasses(await colorizeCode(code, lang))) {
        expect(SYNTAX_CLASSES, `shiki/${lang}: ${name}`).toContain(name);
      }
    }
    for (const [code, language] of LEZER_FIXTURES) {
      for (const name of lezerClasses(code, language)) {
        expect(SYNTAX_CLASSES, `lezer: ${name}`).toContain(name);
      }
    }
  });

  it("reaches every declared bucket from at least one renderer", async () => {
    const reached = new Set<string>();
    for (const [code, lang] of SHIKI_FIXTURES) shikiClasses(await colorizeCode(code, lang)).forEach(name => reached.add(name));
    for (const [code, language] of LEZER_FIXTURES) lezerClasses(code, language).forEach(name => reached.add(name));
    // A bucket nobody can emit is a dead rule wearing a colour. If this fails,
    // either add a fixture that reaches it or delete the bucket — do not add
    // it to an exemption list.
    expect(SYNTAX_CLASSES.filter(name => !reached.has(name))).toEqual([]);
  });
});
