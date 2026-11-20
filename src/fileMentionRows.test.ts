import { describe, expect, it } from "vitest";
import { markSegments, mentionMatch, splitMentionPath } from "./fileMentionRows";

describe("splitMentionPath", () => {
  it("puts a root file's whole path in the name, with no folder", () => {
    expect(splitMentionPath("README.md")).toEqual({ name: "README.md", folder: "" });
  });

  it("splits a nested path at its last slash, without a trailing slash on the folder", () => {
    expect(splitMentionPath("src-tauri/bridge-core/src/policy.rs")).toEqual({ name: "policy.rs", folder: "src-tauri/bridge-core/src" });
    expect(splitMentionPath("src/App.tsx")).toEqual({ name: "App.tsx", folder: "src" });
  });

  it("keeps a dotfile's leading dot in its name", () => {
    expect(splitMentionPath(".github/workflows/ci.yml")).toEqual({ name: "ci.yml", folder: ".github/workflows" });
    expect(splitMentionPath(".gitignore")).toEqual({ name: ".gitignore", folder: "" });
  });
});

describe("mentionMatch", () => {
  it("marks the query in the name, case-insensitively", () => {
    expect(mentionMatch("src/components/PolicyCard.tsx", "pol")).toEqual({ name: [0, 3] });
    expect(mentionMatch("src/components/PolicyCard.tsx", "CARD")).toEqual({ name: [6, 10] });
  });

  it("prefers the name even when the folder matches first", () => {
    // "policy" is in the folder before it is in the name; the name is what the
    // reader is looking at.
    expect(mentionMatch("docs/policy/delegation-policy.md", "policy")).toEqual({ name: [11, 17] });
  });

  it("falls back to the folder when only the folder matches", () => {
    expect(mentionMatch("src-tauri/bridge-core/src/lib.rs", "bridge")).toEqual({ folder: [10, 16] });
  });

  it("splits a match that straddles the slash between the two halves", () => {
    // "auth/tok" covers the end of the folder and the start of the name.
    expect(mentionMatch("src/auth/tokenStore.ts", "auth/tok")).toEqual({ folder: [4, 8], name: [0, 3] });
  });

  it("marks nothing for an empty query or a path the query is not in", () => {
    expect(mentionMatch("src/App.tsx", "")).toEqual({});
    expect(mentionMatch("src/App.tsx", "zzz")).toEqual({});
  });

  it("marks the first occurrence in a root file", () => {
    expect(mentionMatch("README.md", "e")).toEqual({ name: [1, 2] });
  });
});

describe("markSegments", () => {
  it("cuts text into the runs before, inside and after the range", () => {
    expect(markSegments("delegation-policy.md", [11, 14])).toEqual([
      { text: "delegation-", marked: false },
      { text: "pol", marked: true },
      { text: "icy.md", marked: false },
    ]);
  });

  it("drops empty runs at either edge", () => {
    expect(markSegments("policy.rs", [0, 3])).toEqual([{ text: "pol", marked: true }, { text: "icy.rs", marked: false }]);
    expect(markSegments("policy.rs", [6, 9])).toEqual([{ text: "policy", marked: false }, { text: ".rs", marked: true }]);
  });

  it("returns the text unmarked with no range, and nothing for empty text", () => {
    expect(markSegments("src", undefined)).toEqual([{ text: "src", marked: false }]);
    expect(markSegments("", undefined)).toEqual([]);
  });

  it("never indexes past the end of the text", () => {
    expect(markSegments("ab", [1, 9])).toEqual([{ text: "a", marked: false }, { text: "b", marked: true }]);
  });
});
