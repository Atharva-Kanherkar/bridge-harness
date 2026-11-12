import { describe, expect, it } from "vitest";
import { keepUntouched, splitTitleHeading, type BlockCodec } from "./notesBlocks";

// A stand-in for the editor: blocks are separated by blank lines, and its
// style is `-` bullets, `**bold**` and single spaces.
const codec: BlockCodec = {
  split: markdown => {
    const blocks = markdown.match(/[\s\S]*?(?:\n\n+|$)/g)?.filter(Boolean) ?? [];
    return blocks.join("") === markdown ? blocks : null;
  },
  canon: block => block.trim().replace(/^\* /gm, "- ").replace(/__(.+?)__/g, "**$1**").replace(/ {2,}/g, " "),
};
/** What the editor writes for a whole document. */
const write = (markdown: string) => markdown.split(/\n\n+/).map(block => codec.canon(block)).filter(Boolean).join("\n\n");

const source = "Intro  with __bold__.\n\n\n* one\n* two\n\nMiddle para\n\nLast  one\n";

describe("keepUntouched", () => {
  it("splitting loses nothing", () => {
    expect(codec.split(source)!.join("")).toBe(source);
  });

  it("returns an unchanged document byte for byte", () => {
    expect(keepUntouched(source, write(source), codec)).toBe(source);
  });

  it("keeps every block but the edited one as it was", () => {
    const edited = write(source).replace("Middle para", "Middle paragraph");
    expect(keepUntouched(source, edited, codec)).toBe("Intro  with __bold__.\n\n\n* one\n* two\n\nMiddle paragraph\n\nLast  one\n");
  });

  it("keeps untouched blocks through inserts, deletes and moves", () => {
    const blocks = write(source).split("\n\n");
    const inserted = [blocks[0], "New block", ...blocks.slice(1)].join("\n\n");
    expect(keepUntouched(source, inserted, codec)).toBe("Intro  with __bold__.\n\nNew block\n\n* one\n* two\n\nMiddle para\n\nLast  one\n");
    const deleted = [blocks[0], blocks[2], blocks[3]].join("\n\n");
    expect(keepUntouched(source, deleted, codec)).toBe("Intro  with __bold__.\n\nMiddle para\n\nLast  one\n");
    const moved = [blocks[1], blocks[0], blocks[2], blocks[3]].join("\n\n");
    expect(keepUntouched(source, moved, codec)).toBe("* one\n* two\n\nIntro  with __bold__.\n\nMiddle para\n\nLast  one\n");
  });

  it("ends the way the page ended", () => {
    const noNewline = "A\n\nB";
    expect(keepUntouched(noNewline, "A\n\nB\n\nC", codec)).toBe("A\n\nB\n\nC");
    expect(keepUntouched("A\n\nB\n", "A\n\nB\n\nC", codec)).toBe("A\n\nB\n\nC\n");
  });

  it("falls back to the editor's text when the source cannot be split", () => {
    const broken: BlockCodec = { ...codec, split: () => null };
    expect(keepUntouched(source, "x", broken)).toBe("x");
  });
});

describe("splitTitleHeading", () => {
  it("splits off a heading that repeats the title", () => {
    expect(splitTitleHeading("# Launch\n\nBody\n", "Launch")).toEqual({ head: "# Launch\n\n", body: "Body\n" });
    expect(splitTitleHeading("# Launch", " Launch ")).toEqual({ head: "# Launch", body: "" });
    expect(splitTitleHeading("# Launch\nBody", "Launch")).toEqual({ head: "# Launch\n", body: "Body" });
  });
  it("leaves anything else alone", () => {
    expect(splitTitleHeading("# Other\n\nBody", "Launch")).toEqual({ head: "", body: "# Other\n\nBody" });
    expect(splitTitleHeading("## Launch\n", "Launch")).toEqual({ head: "", body: "## Launch\n" });
    expect(splitTitleHeading("# Launch\n", null)).toEqual({ head: "", body: "# Launch\n" });
  });
});
