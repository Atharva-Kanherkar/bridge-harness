import { afterEach, describe, expect, it, vi } from "vitest";
import { colorizeCode, colorizePatch, highlightPatch, languageFromPath, normalizeLang, looksLikeDiff } from "./highlight";

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
  it("resolves common aliases to their canonical Shiki id", () => {
    expect(normalizeLang("ts")).toBe("typescript");
    expect(normalizeLang("yml")).toBe("yaml");
    expect(normalizeLang("html")).toBe("xml");
  });

  it("remaps objective-c's aliases to Shiki's hyphenated id", () => {
    expect(normalizeLang("objectivec")).toBe("objective-c");
    expect(normalizeLang("objc")).toBe("objective-c");
    expect(normalizeLang("m")).toBe("objective-c");
  });

  it("gives toml, json5 and jsonc their own grammars instead of aliasing", () => {
    expect(normalizeLang("toml")).toBe("toml");
    expect(normalizeLang("json5")).toBe("json5");
    expect(normalizeLang("jsonc")).toBe("jsonc");
  });

  it("rejects unknown languages and the explicit plain markers", () => {
    expect(normalizeLang("not-a-real-language")).toBe("");
    expect(normalizeLang("plaintext")).toBe("");
    expect(normalizeLang("text")).toBe("");
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
