import { afterEach, describe, expect, it, vi } from "vitest";
import { colorizeCode, colorizePatch, highlightPatch, languageFromPath, normalizeLang, looksLikeDiff, SYNTAX_CLASSES } from "./highlight";
import { EDITOR_LANGUAGES } from "./editor/language";

describe("languageFromPath", () => {
  it("maps common extensions", () => {
    expect(languageFromPath("src/App.tsx")).toBe("typescript");
    expect(languageFromPath("src-tauri/src/main.rs")).toBe("rust");
    expect(languageFromPath("scripts/run.sh")).toBe("bash");
    expect(languageFromPath("app/models/user.rb")).toBe("ruby");
    expect(languageFromPath("Config.kt")).toBe("kotlin");
    expect(languageFromPath("api/schema.graphql")).toBe("graphql");
  });

  it("recognises extensionless conventions and dotfiles", () => {
    expect(languageFromPath("Dockerfile")).toBe("dockerfile");
    expect(languageFromPath("deploy/Makefile")).toBe("makefile");
    expect(languageFromPath(".env")).toBe("ini");
  });

  it("falls through to the extension underneath a suffix", () => {
    expect(languageFromPath("config.yaml.tmpl")).toBe("yaml");
  });

  it("returns empty for anything we have no grammar for", () => {
    expect(languageFromPath("assets/logo.png")).toBe("");
    expect(languageFromPath("")).toBe("");
  });
});

describe("normalizeLang", () => {
  it("resolves common aliases to their canonical id", () => {
    expect(normalizeLang("ts")).toBe("typescript");
    expect(normalizeLang("yml")).toBe("yaml");
    expect(normalizeLang("html")).toBe("xml");
  });

  // The public vocabulary is a shared contract with editor/language.ts's
  // LOADERS map (keyed by exactly these ids) — it must not change even
  // though highlight.ts internally loads a better Shiki grammar for some of
  // them. See the "uses a better Shiki grammar than the public id implies"
  // describe block below for that internal behaviour.
  it("keeps objective-c's aliases on the original public id", () => {
    expect(normalizeLang("objectivec")).toBe("objectivec");
    expect(normalizeLang("objc")).toBe("objectivec");
    expect(normalizeLang("m")).toBe("objectivec");
  });

  it("keeps toml, json5 and jsonc aliased to their original public id", () => {
    expect(normalizeLang("toml")).toBe("ini");
    expect(normalizeLang("json5")).toBe("json");
    expect(normalizeLang("jsonc")).toBe("json");
  });

  it("rejects unknown languages and the explicit plain markers", () => {
    expect(normalizeLang("not-a-real-language")).toBe("");
    expect(normalizeLang("plaintext")).toBe("");
    expect(normalizeLang("text")).toBe("");
  });

  // editor/language.ts's LOADERS map is keyed by exactly what this function
  // produces (its own doc comment says so) — this PR once broke that by
  // renaming objectivec/ini/json's public id to match Shiki's own grammar
  // ids, silently disabling the editor's language support for .m/.mm/.toml/
  // .jsonc/.json5 without touching editor/language.ts at all. Guard the
  // exact ids SHIKI_GRAMMAR_FOR redirects internally, so a future change
  // that reintroduces the same mistake fails here instead of shipping.
  it("keeps every SHIKI_GRAMMAR_FOR-touched id resolvable by the editor's own loader map", () => {
    const touchedInputs = ["objectivec", "objc", "m", "mm", "toml", "json5", "jsonc", "javascript", "js", "jsx", "typescript", "ts", "tsx"];
    for (const input of touchedInputs) {
      const id = normalizeLang(input);
      expect(id).not.toBe("");
      expect(EDITOR_LANGUAGES).toContain(id);
    }
  });
});

describe("uses a better Shiki grammar than the public id implies", () => {
  it("colours a JSX component tag instead of misreading it as a comparison", async () => {
    const html = await colorizeCode("const el = <Button>Hi</Button>;", "tsx");
    expect(html).toContain("stx-tag");
  });

  it("still colours plain, JSX-free TypeScript correctly via the same grammar", async () => {
    const html = await colorizeCode("const x: number = 1;", "typescript");
    expect(html).toContain("stx-keyword");
    expect(html).toContain("stx-number");
  });

  it("colours Objective-C despite normalizeLang reporting the old public id", async () => {
    expect(normalizeLang("objectivec")).toBe("objectivec"); // the contract, restated
    const html = await colorizeCode("@interface Foo : NSObject\n@end", "objectivec");
    expect(html).toContain("stx-");
  });

  it("colours a TOML-shaped .env-like file despite normalizeLang reporting ini", async () => {
    expect(normalizeLang("toml")).toBe("ini"); // the contract, restated
    const html = await colorizeCode('name = "bridge"\n[package]\nversion = "1.0"', "toml");
    expect(html).toContain("stx-");
  });
});

describe("looksLikeDiff", () => {
  it("recognises unified diff shapes and rejects ordinary text", () => {
    expect(looksLikeDiff("@@ -1,2 +1,2 @@\n-old\n+new")).toBe(true);
    expect(looksLikeDiff("just some prose about a project")).toBe(false);
    expect(looksLikeDiff("")).toBe(false);
  });
});

describe("colorizeCode", () => {
  it("wraps recognized tokens in .stx-* classes", async () => {
    const html = await colorizeCode("const x = 1;", "typescript");
    expect(html).toContain("stx-keyword");
  });

  it("colours a comment distinctly from code", async () => {
    const html = await colorizeCode("// just a comment", "typescript");
    expect(html).toContain("stx-comment");
  });

  it("escapes and returns plain text for a language with no grammar", async () => {
    expect(await colorizeCode("<script>x</script>", "notarealtoollang")).toBe("&lt;script&gt;x&lt;/script&gt;");
  });

  it("escapes and returns plain text past the size cap", async () => {
    const big = "const value = 1;\n".repeat(10_000); // well past MAX_HIGHLIGHT_CHARS
    const html = await colorizeCode(big, "typescript");
    expect(html).not.toContain("stx-");
    expect(html).toBe(
      big.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"),
    );
  });
});

const PATCH = [
  "diff --git a/src/sum.ts b/src/sum.ts",
  "index 111..222 100644",
  "--- a/src/sum.ts",
  "+++ b/src/sum.ts",
  "@@ -4,3 +4,4 @@ export function sum(values: number[]) {",
  " const total = 0;",
  "-  return total;",
  "+  const doubled = total * 2;",
  "+  return doubled;",
].join("\n");

describe("highlightPatch (sync, plain)", () => {
  it("classifies headers, hunks and content", () => {
    const rows = highlightPatch(PATCH, "src/sum.ts");
    expect(rows.map(row => row.kind)).toEqual([
      "meta", "meta", "meta", "meta", "hunk", "context", "del", "add", "add",
    ]);
  });

  it("numbers both sides from the hunk header", () => {
    const rows = highlightPatch(PATCH, "src/sum.ts");
    const numbers = rows.slice(5).map(row => [row.oldLine, row.newLine]);
    expect(numbers).toEqual([[4, 4], [5, null], [null, 5], [null, 6]]);
  });

  it("strips the marker but never colours — that is colorizePatch's job", () => {
    const rows = highlightPatch(PATCH, "src/sum.ts");
    const added = rows.find(row => row.kind === "add");
    expect(added?.html.startsWith("+")).toBe(false);
    expect(added?.html).not.toContain("stx-");
    expect(added?.html).toBe("  const doubled = total * 2;");
  });

  it("leaves bodies plain but escaped for unknown languages", () => {
    const rows = highlightPatch("@@ -1 +1 @@\n+<script>x</script>", "notes.unknownext");
    expect(rows[1].html).toBe("&lt;script&gt;x&lt;/script&gt;");
  });

  it("still classifies a bare fragment with no hunk header", () => {
    const rows = highlightPatch("-const a = 1;\n+const a = 2;", "a.ts");
    expect(rows.map(row => row.kind)).toEqual(["del", "add"]);
    expect(rows.every(row => row.oldLine === null && row.newLine === null)).toBe(true);
  });

  it("treats '---' inside a hunk as a deletion, not a header", () => {
    const rows = highlightPatch("@@ -1 +1 @@\n--- dashes\n+++ dashes", "notes.md");
    expect(rows.map(row => row.kind)).toEqual(["hunk", "del", "add"]);
  });

  it("returns nothing for an empty patch", () => {
    expect(highlightPatch("", "a.ts")).toEqual([]);
  });

  const TWO_FILES = [
    "diff --git a/a.ts b/a.ts", "index 1..2 100644", "--- a/a.ts", "+++ b/a.ts",
    "@@ -1,2 +1,2 @@", " const x = 1;", "-const y = 2;", "+const y = 3;",
    "diff --git a/b.ts b/b.ts", "--- a/b.ts", "+++ b/b.ts",
    "@@ -1 +1 @@", "-old", "+new",
  ].join("\n");

  it("keeps every file's headers intact in a multi-file patch", () => {
    const rows = highlightPatch(TWO_FILES, "a.ts");
    expect(rows.map(row => row.kind)).toEqual([
      "meta", "meta", "meta", "meta", "hunk", "context", "del", "add",
      "meta", "meta", "meta", "hunk", "del", "add",
    ]);
    expect(rows[8].html).toBe("diff --git a/b.ts b/b.ts");
    expect(rows[9].html).toBe("--- a/b.ts");
    expect(rows[10].html).toBe("+++ b/b.ts");
  });

  it("numbers the second file from its own hunk header", () => {
    const rows = highlightPatch(TWO_FILES, "a.ts");
    expect([rows[12].oldLine, rows[12].newLine]).toEqual([1, null]);
    expect([rows[13].oldLine, rows[13].newLine]).toEqual([null, 1]);
  });

  it("recovers when a hunk header undercounts its own lines", () => {
    const rows = highlightPatch("@@ -1 +1 @@\n-a\n-b\n-c\ndiff --git a/z.ts b/z.ts", "a.ts");
    expect(rows[rows.length - 1]).toMatchObject({ kind: "meta", html: "diff --git a/z.ts b/z.ts" });
  });

  it("leaves prose around a fragment whole rather than eating its first character", () => {
    const rows = highlightPatch("Success updating foo.ts\n-old\n+new", "foo.ts");
    expect(rows[0]).toMatchObject({ kind: "meta", html: "Success updating foo.ts" });
    expect(rows.map(row => row.kind)).toEqual(["meta", "del", "add"]);
  });

  it("does not count the no-newline marker against the hunk", () => {
    const rows = highlightPatch("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new", "a.ts");
    expect(rows.map(row => row.kind)).toEqual(["hunk", "del", "meta", "add"]);
  });
});

describe("colorizePatch (async, coloured)", () => {
  it("highlights bodies in the file's language and strips the marker", async () => {
    const rows = await colorizePatch(PATCH, "src/sum.ts");
    const added = rows.find(row => row.kind === "add");
    expect(added?.html).toContain("stx-keyword");
    expect(added?.html.startsWith("+")).toBe(false);
  });

  it("keeps multi-line constructs intact across added lines", async () => {
    const rows = await colorizePatch("@@ -1,0 +1,2 @@\n+/* one\n+   two */", "a.ts");
    expect(rows[1].html).toContain("stx-comment");
    expect(rows[2].html).toContain("stx-comment");
  });

  it("colours a blank context line without losing row alignment", async () => {
    const rows = await colorizePatch("@@ -1,3 +1,3 @@\n const a = 1;\n \n const b = 2;", "a.ts");
    expect(rows).toHaveLength(4); // hunk header + 3 content lines
    expect(rows[2].html).toBe("");
  });

  it("leaves bodies plain but escaped for unknown languages", async () => {
    const rows = await colorizePatch("@@ -1 +1 @@\n+<script>x</script>", "notes.unknownext");
    expect(rows[1].html).toBe("&lt;script&gt;x&lt;/script&gt;");
  });

  it("skips highlighting a patch past the size cap", async () => {
    const filler = "+const value = 1;".repeat(30_000);
    const rows = await colorizePatch(`@@ -1 +1 @@\n${filler}\n+const a = 1;`, "a.ts");
    expect(rows.at(-1)!.html).toBe("const a = 1;");
  });

  it("returns nothing for an empty patch", async () => {
    expect(await colorizePatch("", "a.ts")).toEqual([]);
  });
});

describe("colorization failure fallbacks", () => {
  afterEach(() => {
    vi.doUnmock("shiki/core");
    vi.resetModules();
  });

  it("falls back to escaped text when the highlighter itself fails to load", async () => {
    vi.resetModules();
    vi.doMock("shiki/core", async () => {
      const real = await vi.importActual<typeof import("shiki/core")>("shiki/core");
      return {
        ...real,
        createHighlighterCore: () => Promise.reject(new Error("engine exploded")),
      };
    });
    const { colorizeCode: freshColorizeCode } = await import("./highlight");
    expect(await freshColorizeCode("const a = 1;", "typescript")).toBe("const a = 1;");
  });

  it("falls back to escaped text when highlighting returns the wrong number of lines", async () => {
    vi.resetModules();
    vi.doMock("shiki/core", async () => {
      const real = await vi.importActual<typeof import("shiki/core")>("shiki/core");
      const core = await real.createHighlighterCore({
        engine: (await import("shiki/engine/javascript")).createJavaScriptRegexEngine(),
        themes: [import("shiki/themes/github-dark.mjs")],
        langs: [import("shiki/langs/typescript.mjs")],
      });
      vi.spyOn(core, "codeToTokens").mockReturnValue({ tokens: [[]], fg: "", bg: "", themeName: "" } as never);
      return { ...real, createHighlighterCore: () => Promise.resolve(core) };
    });
    const { colorizePatch: freshColorizePatch } = await import("./highlight");
    const rows = await freshColorizePatch("@@ -1,2 +1,2 @@\n+const a = 1;\n+const b = 2;", "a.ts");
    expect(rows.slice(1).map(row => row.html)).toEqual(["const a = 1;", "const b = 2;"]);
  });
});

/**
 * Class applied to an exact token, or `null` when the token is emitted with
 * no class. Asserting on a named token rather than `toContain("stx-x")`
 * anywhere in the line is the difference between these tests and the first
 * version of them: `stx-regex` "passing" while the delimiters came out
 * string-green, and `stx-operator` "passing" off the `=` while `=>` stayed
 * keyword-violet, were both invisible to a substring check.
 */
function classOf(html: string, token: string): string | null {
  const escaped = token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const wrapped = new RegExp(`<span class="(stx-[a-z]+)">${escaped}</span>`).exec(html);
  if (wrapped) return wrapped[1];
  // Present but unwrapped (whitespace, or a scope with no rule).
  return html.includes(`>${token}<`) || html.includes(token) ? null : "MISSING";
}

describe("the widened scope vocabulary", () => {
  // Each of these buckets existed in `index.css` (or was about to) with no
  // rule in `SCOPE_RULES` able to reach it, so the tokens rendered as bare
  // text. They are the bulk of any real file, which is why code read flat.
  it("colours an identifier and its member separately inside one merged token", async () => {
    // `" items."` arrives from Shiki as a *single* token spanning whitespace,
    // an identifier and an accessor, because Shiki merges adjacent
    // same-styled runs. Colouring per token painted `items` punctuation-grey.
    const html = await colorizeCode("const n = items.length;", "typescript");
    expect(classOf(html, "items")).toBe("stx-variable");
    expect(classOf(html, "length")).toBe("stx-property");
    expect(classOf(html, ".")).toBe("stx-punct");
  });

  it("colours an assigned member as a property", async () => {
    const html = await colorizeCode("obj.prop = 1;", "typescript");
    expect(classOf(html, "prop")).toBe("stx-property");
  });

  it("colours a JSX attribute name as a property and the element as a tag", async () => {
    const html = await colorizeCode('<div className="x" />', "tsx");
    expect(classOf(html, "className")).toBe("stx-property");
    expect(classOf(html, "div")).toBe("stx-tag");
  });

  it("gives operators their own bucket, including the arrow", async () => {
    // `=>` is scoped `storage.type.function.arrow`, so it used to land in
    // keyword-violet while the PR claimed it was an operator.
    const html = await colorizeCode("const f = (a, b) => a + b;", "typescript");
    expect(classOf(html, "=&gt;")).toBe("stx-operator");
    expect(classOf(html, "+")).toBe("stx-operator");
    expect(classOf(html, ";")).toBe("stx-punct");
  });

  it("paints a regex literal in exactly one colour", async () => {
    // Delimiters carry `punctuation.definition.string.*`, the quantifier
    // carries `keyword.operator.quantifier.regexp` and the flags carry
    // `keyword.other` — three different buckets for one literal until the
    // `string.regexp` container rule.
    const html = await colorizeCode("const re = /a+b/g;", "typescript");
    const classes = [...html.matchAll(/<span class="(stx-[a-z]+)">([^<]*)<\/span>/g)]
      .filter(match => "/a+b/g".includes(match[2]) && match[2].length > 0 && !"const re = ;".includes(match[2]))
      .map(match => match[1]);
    expect(new Set(classes)).toEqual(new Set(["stx-regex"]));
  });

  it("colours an escape inside a string as an escape", async () => {
    const html = await colorizeCode('const s = "a\\nb";', "typescript");
    expect(classOf(html, "\\n")).toBe("stx-regex");
    expect(classOf(html, "a")).toBe("stx-string");
  });

  it("colours a parameter distinctly at its declaration and its use", async () => {
    const html = await colorizeCode("function f(alpha) { return alpha; }", "typescript");
    expect(classOf(html, "alpha")).toBe("stx-params");
    // The *use* of `alpha` sits inside the merged token `" alpha; }"`.
    expect(html.match(/class="stx-variable">alpha</)).toBeTruthy();
  });

  it("keeps `this` a keyword and the member after it a property", async () => {
    const html = await colorizeCode("class A { m() { return this.x; } }", "typescript");
    expect(classOf(html, "this")).toBe("stx-keyword");
    expect(classOf(html, "x")).toBe("stx-property");
  });

  it("leaves a `const` binding as an identifier, not a literal", async () => {
    // Regression guard. TextMate scopes every `const` name
    // `variable.other.constant`; bucketing that as a number painted most of a
    // TypeScript file amber.
    const html = await colorizeCode("const total = 2;", "typescript");
    expect(classOf(html, "total")).toBe("stx-variable");
  });

  it("does not paint the receiver of a call as a function", async () => {
    // `meta.function-call` is a *range* scope covering `obj.trim()`, so a rule
    // for it labelled `obj` function-blue.
    const html = await colorizeCode("obj.trim();", "typescript");
    expect(classOf(html, "obj")).toBe("stx-variable");
    expect(classOf(html, "trim")).toBe("stx-function");
  });

  it("reaches markdown emphasis, which the delimiters used to shadow", async () => {
    // `"**bold**"` is one token whose last scope is the closing
    // `punctuation.definition.bold`, so bold/italic/strike were unreachable.
    const html = await colorizeCode("**bold** and ~~gone~~ and *soft*", "markdown");
    expect(classOf(html, "bold")).toBe("stx-strong");
    expect(classOf(html, "gone")).toBe("stx-strike");
    expect(classOf(html, "soft")).toBe("stx-emphasis");
  });

  it("emits only classes that are in the shared vocabulary", async () => {
    const samples: [string, string][] = [
      ["const x: Foo = bar.baz(1, /re/g);", "typescript"],
      ["<App title={x} />", "tsx"],
      ["def f(a, b=2):\n  return a # note", "python"],
      ["fn main() { let v: Vec<u8> = vec![]; }", "rust"],
      ["# Title\n\n**bold** and `code`\n", "markdown"],
      ['{"a": 1, "b": [true, null]}', "json"],
      ["body { color: #fff; }", "css"],
      ["SELECT a FROM t WHERE b = 1;", "sql"],
    ];
    for (const [code, lang] of samples) {
      const html = await colorizeCode(code, lang);
      for (const name of html.match(/class="(stx-[a-z]+)"/g) ?? []) {
        const cls = name.slice(7, -1);
        expect(SYNTAX_CLASSES, `${lang}: ${cls}`).toContain(cls);
      }
    }
  });

  it("never drops or duplicates a character while splitting a token", async () => {
    // The per-entry split is the one change here that could corrupt source.
    for (const [code, lang] of [
      ["const f = (a: T) => `x${a}y` + /re/g; // c", "typescript"],
      ["<div a=\"1\">{x}</div>", "tsx"],
      ["**b** ~~s~~ [l](u)", "markdown"],
    ] as [string, string][]) {
      const html = await colorizeCode(code, lang);
      const text = html.replace(/<[^>]+>/g, "")
        .replace(/&quot;/g, '"').replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&amp;/g, "&");
      expect(text, lang).toBe(code);
    }
  });
});
