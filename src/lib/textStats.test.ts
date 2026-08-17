import { describe, expect, it } from "vitest";
import {
  averageWordLength,
  countLines,
  countSentences,
  countWords,
  getTextStats,
  slugify,
  titleCase,
  truncateWithEllipsis,
} from "./textStats";

describe("countWords", () => {
  it("counts words separated by whitespace", () => {
    expect(countWords("hello world")).toBe(2);
  });

  it("returns 0 for empty input", () => {
    expect(countWords("   ")).toBe(0);
  });
});

describe("countLines", () => {
  it("counts newline-separated lines", () => {
    expect(countLines("a\nb\nc")).toBe(3);
  });

  it("returns 0 for empty string", () => {
    expect(countLines("")).toBe(0);
  });
});

describe("countSentences", () => {
  it("counts sentence terminators", () => {
    expect(countSentences("Hi there. How are you? Great!")).toBe(3);
  });

  it("treats text without terminators as one sentence", () => {
    expect(countSentences("no terminator here")).toBe(1);
  });
});

describe("averageWordLength", () => {
  it("computes average length ignoring punctuation", () => {
    expect(averageWordLength("cat. dog!")).toBe(3);
  });

  it("returns 0 for empty text", () => {
    expect(averageWordLength("")).toBe(0);
  });
});

describe("getTextStats", () => {
  it("aggregates all stats", () => {
    const stats = getTextStats("Hi there.\nBye.");
    expect(stats.words).toBe(3);
    expect(stats.lines).toBe(2);
    expect(stats.sentences).toBe(2);
  });
});

describe("truncateWithEllipsis", () => {
  it("leaves short text untouched", () => {
    expect(truncateWithEllipsis("short", 10)).toBe("short");
  });

  it("truncates and appends ellipsis", () => {
    expect(truncateWithEllipsis("hello world", 6)).toBe("hello…");
  });

  it("throws on negative maxLength", () => {
    expect(() => truncateWithEllipsis("x", -1)).toThrow();
  });
});

describe("slugify", () => {
  it("converts spaces and punctuation into hyphens", () => {
    expect(slugify("Hello, World!")).toBe("hello-world");
  });
});

describe("titleCase", () => {
  it("capitalizes each word", () => {
    expect(titleCase("the quick brown fox")).toBe("The Quick Brown Fox");
  });
});
