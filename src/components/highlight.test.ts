import { describe, expect, it, vi } from "vitest";
import hljs from "highlight.js/lib/core";
import { highlightPatch, languageFromPath, splitHighlightedLines } from "./highlight";

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

describe("splitHighlightedLines", () => {
  it("re-opens spans that straddle a newline", () => {
    const lines = splitHighlightedLines('<span class="hljs-comment">/* one\ntwo */</span> tail');
    expect(lines).toEqual([
      '<span class="hljs-comment">/* one</span>',
      '<span class="hljs-comment">two */</span> tail',
    ]);
  });

  it("keeps plain text intact", () => {
    expect(splitHighlightedLines("a\nb")).toEqual(["a", "b"]);
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

describe("highlightPatch", () => {
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

  it("highlights bodies in the file's language and strips the marker", () => {
    const rows = highlightPatch(PATCH, "src/sum.ts");
    const added = rows.find(row => row.kind === "add");
    expect(added?.html).toContain("hljs-keyword");
    expect(added?.html.startsWith("+")).toBe(false);
  });

  it("leaves bodies plain but escaped for unknown languages", () => {
    const rows = highlightPatch("@@ -1 +1 @@\n+<script>x</script>", "notes.unknownext");
    expect(rows[1].html).toBe("&lt;script&gt;x&lt;/script&gt;");
  });

  it("keeps multi-line constructs intact across added lines", () => {
    const rows = highlightPatch("@@ -1,0 +1,2 @@\n+/* one\n+   two */", "a.ts");
    expect(rows[1].html).toContain("hljs-comment");
    expect(rows[2].html).toContain("hljs-comment");
  });

  it("still colours a bare fragment with no hunk header", () => {
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
    // The hunk's own line counts are what end it; without them the second
    // file's headers are read as code and lose their first character.
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

  it("skips highlighting a patch past the size cap", () => {
    const filler = "+const value = 1;".repeat(30_000);
    const rows = highlightPatch(`@@ -1 +1 @@\n${filler}\n+const a = 1;`, "a.ts");
    expect(rows.at(-1)!.html).toBe("const a = 1;");
  });

  it("falls back to escaped text when the grammar throws", () => {
    const spy = vi.spyOn(hljs, "highlight").mockImplementation(() => { throw new Error("grammar exploded"); });
    try {
      expect(highlightPatch("@@ -1 +1 @@\n+const a = 1;", "a.ts").at(-1)!.html).toBe("const a = 1;");
    } finally {
      spy.mockRestore();
    }
  });

  it("falls back when highlighting returns the wrong number of lines", () => {
    const spy = vi.spyOn(hljs, "highlight").mockReturnValue({ value: "only one line" } as never);
    try {
      const rows = highlightPatch("@@ -1,2 +1,2 @@\n+const a = 1;\n+const b = 2;", "a.ts");
      expect(rows.slice(1).map(row => row.html)).toEqual(["const a = 1;", "const b = 2;"]);
    } finally {
      spy.mockRestore();
    }
  });
});
