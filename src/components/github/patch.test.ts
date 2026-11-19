import { describe, expect, it } from "vitest";
import { commentAnchor, hunkTail, parsePatch } from "./patch";

describe("parsePatch", () => {
  it("numbers each row against the file it belongs to", () => {
    const lines = parsePatch("@@ -10,3 +20,4 @@ fn main() {\n keep\n-old\n+new\n+newer\n tail");
    expect(lines.map(line => [line.kind, line.oldLine, line.newLine])).toEqual([
      ["hunk", undefined, undefined],
      ["context", 10, 20],
      ["del", 11, undefined],
      ["add", undefined, 21],
      ["add", undefined, 22],
      ["context", 12, 23],
    ]);
  });

  it("restarts the count at every hunk and ignores git's own markers", () => {
    const lines = parsePatch("@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n@@ -40,2 +40,2 @@\n x\n-y\n+z");
    expect(lines[3].kind).toBe("meta");
    expect(lines.slice(5).map(line => [line.oldLine, line.newLine])).toEqual([[40, 40], [41, undefined], [undefined, 41]]);
  });

  it("treats text before the first hunk as belonging to no line", () => {
    expect(parsePatch("+line 0\n+line 1").every(line => line.kind === "meta")).toBe(true);
  });
});

describe("commentAnchor", () => {
  it("puts a deleted line on the old file and everything else on the new one", () => {
    const [hunk, context, removed, added] = parsePatch("@@ -5,2 +5,2 @@\n keep\n-old\n+new");
    expect(commentAnchor(hunk)).toBeNull();
    expect(commentAnchor(context)).toEqual({ line: 5, side: "right" });
    expect(commentAnchor(removed)).toEqual({ line: 6, side: "left" });
    expect(commentAnchor(added)).toEqual({ line: 6, side: "right" });
  });
});

describe("hunkTail", () => {
  it("keeps the last lines of a thread's hunk, without the header", () => {
    const tail = hunkTail("@@ -1,6 +1,6 @@\n a\n b\n c\n d\n-e\n+f", 3);
    expect(tail.map(line => line.text)).toEqual([" d", "-e", "+f"]);
    expect(tail[2].newLine).toBe(5);
  });
});
