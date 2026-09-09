import { describe, expect, it } from "vitest";
import { FileLock, FileText, Folder, GitBranch, Package } from "lucide-react";
import { glyphFor } from "./fileGlyph";
import { languageFromPath } from "./highlight";

describe("glyphFor", () => {
  it("routes a file through languageFromPath rather than its own extension table", () => {
    // The point of the shared detection path: if the glyph and the
    // highlighter can disagree, the tree starts lying about what a file is.
    // `.mts` is only TypeScript because `LANG_ALIASES` says so.
    expect(languageFromPath("src/thing.mts")).toBe("typescript");
    expect(glyphFor("src/thing.mts")).toEqual(glyphFor("src/thing.ts"));
    expect(glyphFor("src/App.tsx")).toEqual(glyphFor("src/api.ts"));
  });

  it("gives distinct tints to distinct language families", () => {
    const ts = glyphFor("src/api.ts").tint;
    const rs = glyphFor("src-tauri/src/main.rs").tint;
    const py = glyphFor("scripts/build.py").tint;
    expect(new Set([ts, rs, py]).size).toBe(3);
  });

  it("prefers a filename role over the file's grammar", () => {
    // package.json and package-lock.json are both JSON; only one is worth
    // opening, and the tree should say so.
    expect(glyphFor("package.json").Icon).toBe(Package);
    expect(glyphFor("package-lock.json").Icon).toBe(FileLock);
    expect(glyphFor("bun.lock").Icon).toBe(FileLock);
    expect(glyphFor(".gitignore").Icon).toBe(GitBranch);
  });

  it("matches filename roles case-insensitively and by basename", () => {
    expect(glyphFor("apps/web/Package.json").Icon).toBe(Package);
    expect(glyphFor("src-tauri/Cargo.toml").Icon).toBe(Package);
  });

  it("falls back to a plain sheet for anything unrecognized", () => {
    expect(glyphFor("notes/whatever.qqq")).toEqual({ Icon: FileText, tint: "text-syn-punct" });
    expect(glyphFor("")).toEqual({ Icon: FileText, tint: "text-syn-punct" });
  });

  it("covers extensions that have no grammar but an obvious role", () => {
    expect(languageFromPath("assets/logo.png")).toBe(""); // no grammar
    expect(glyphFor("assets/logo.png").Icon).not.toBe(FileText);
    expect(glyphFor("data/rows.csv").Icon).not.toBe(FileText);
  });

  it("only ever tints from the syntax ramp", () => {
    const paths = [
      "src/api.ts", "src/App.tsx", "main.rs", "build.py", "go.mod", "a.rb",
      "A.java", "b.kt", "c.swift", "d.c", "e.cpp", "f.cs", "g.m", "h.dart",
      "i.ex", "j.scala", "k.php", "l.lua", "m.pl", "n.json", "o.yaml",
      "p.toml", "q.xml", "r.graphql", "s.proto", "t.sql", "u.css", "v.scss",
      "w.md", "x.diff", "y.sh", "Makefile", "Dockerfile", "package.json",
      "bun.lock", ".gitignore", "logo.png", "rows.csv", "z.wasm", "q.qqq",
    ];
    for (const path of paths) {
      const { tint, Icon } = glyphFor(path);
      expect(tint, path).toMatch(/^text-syn-[a-z]+$/);
      expect(Icon, path).toBeTruthy();
    }
  });

  it("is not used for directories — those carry a folder glyph in the row", () => {
    // Guard on intent: `glyphFor` takes a file path only, and a directory
    // named like a file must not silently acquire a code glyph in the tree.
    // The tree passes directories through `Folder`/`FolderOpen` instead.
    expect(Folder).toBeTruthy();
    expect(glyphFor("src/components").Icon).toBe(FileText);
  });
});
