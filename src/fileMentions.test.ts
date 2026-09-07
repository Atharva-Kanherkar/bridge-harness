import { describe, expect, it } from "vitest";
import { appendFileMention, applyFileMention, fileMentionQuery, formatFileMention } from "./fileMentions";

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

  it("appends a picked file without disturbing the draft", () => {
    // The user chose this from a dialog, so nothing is being replaced — their
    // words stay and the reference is added.
    expect(appendFileMention("look at this", "/Users/me/notes.md"))
      .toBe("look at this @/Users/me/notes.md ");
    // Spacing is handled rather than doubled.
    expect(appendFileMention("look at this ", "/tmp/a.txt")).toBe("look at this @/tmp/a.txt ");
    expect(appendFileMention("", "/tmp/a.txt")).toBe("@/tmp/a.txt ");
    // Several files in one pick, each a separate reference.
    const two = ["/tmp/a.txt", "/Users/me/My Notes/b.md"].reduce(appendFileMention, "compare");
    expect(two).toBe(`compare @/tmp/a.txt @${JSON.stringify("/Users/me/My Notes/b.md")} `);
  });

  it("keeps an absolute path with spaces addressable", () => {
    const path = "/Users/me/Desktop/quarterly report.pdf";
    expect(formatFileMention(path)).toBe(`@${JSON.stringify(path)}`);
    expect(fileMentionQuery(`see @${JSON.stringify(path).slice(0, 12)}`)).toBeDefined();
  });

  it("reads active quoted and unquoted queries without matching emails", () => {
    expect(fileMentionQuery("review @src/Ap")).toBe("src/Ap");
    expect(fileMentionQuery('review @"design sp')).toBe("design sp");
    expect(fileMentionQuery('review @"a\\\"b')).toBe('a"b');
    expect(fileMentionQuery("email name@host")).toBeUndefined();
  });
});
