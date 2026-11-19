// A unified patch, line by line, with the file line each row belongs to. The
// Changes tab needs the numbers twice: to print a gutter, and to say where a
// new review comment goes. GitHub anchors a line comment to (path, line, side),
// so a row that has no line on either side (a hunk header, the "no newline"
// marker) is not a place a comment can go.

export type PatchLineKind = "hunk" | "add" | "del" | "context" | "meta";

export type PatchLine = {
  text: string;
  kind: PatchLineKind;
  /** The line in the old file; absent on an added line. */
  oldLine?: number;
  /** The line in the new file; absent on a deleted line. */
  newLine?: number;
};

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export function parsePatch(patch: string): PatchLine[] {
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;
  return patch.split("\n").map((text): PatchLine => {
    const hunk = HUNK.exec(text);
    if (hunk) {
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      inHunk = true;
      return { text, kind: "hunk" };
    }
    // Anything before the first hunk header, and git's own "\ No newline"
    // marker, belongs to no line.
    if (!inHunk || text.startsWith("\\")) return { text, kind: "meta" };
    if (text.startsWith("+")) return { text, kind: "add", newLine: newLine++ };
    if (text.startsWith("-")) return { text, kind: "del", oldLine: oldLine++ };
    return { text, kind: "context", oldLine: oldLine++, newLine: newLine++ };
  });
}

/** Where a review comment on this row goes, or `null` when it cannot take one.
 *  A deleted line is the old file's; everything else is the new file's. */
export function commentAnchor(line: PatchLine): { line: number; side: "left" | "right" } | null {
  if (line.kind === "del" && line.oldLine !== undefined) return { line: line.oldLine, side: "left" };
  if ((line.kind === "add" || line.kind === "context") && line.newLine !== undefined) return { line: line.newLine, side: "right" };
  return null;
}

/** The last `count` rows of a review thread's hunk: the lines the comment is
 *  actually about. GitHub shows the same tail. */
export function hunkTail(hunk: string, count = 4): PatchLine[] {
  const lines = parsePatch(hunk);
  const body = lines.filter(line => line.kind !== "hunk" && line.kind !== "meta");
  return body.slice(-count);
}
