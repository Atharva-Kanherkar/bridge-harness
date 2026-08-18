import { describe, expect, it } from "vitest";
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
});
