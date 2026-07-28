import { describe, expect, it } from "vitest";
import { applyFileMention, fileMentionQuery, formatFileMention } from "./fileMentions";

describe("file mentions", () => {
  it("keeps simple paths readable", () => {
    expect(formatFileMention("src/App.tsx")).toBe("@src/App.tsx");
    expect(applyFileMention("review @App", "src/App.tsx")).toBe("review @src/App.tsx ");
  });

  it("JSON-quotes paths with spaces, Unicode, or punctuation", () => {
    const path = "文档/design spec #1.md";
    expect(formatFileMention(path)).toBe(`@${JSON.stringify(path)}`);
    expect(applyFileMention("review @des", path)).toBe(`review @${JSON.stringify(path)} `);
  });

  it("reads active quoted and unquoted queries without matching emails", () => {
    expect(fileMentionQuery("review @src/Ap")).toBe("src/Ap");
    expect(fileMentionQuery('review @"design sp')).toBe("design sp");
    expect(fileMentionQuery('review @"a\\\"b')).toBe('a"b');
    expect(fileMentionQuery("email name@host")).toBeUndefined();
  });
});
