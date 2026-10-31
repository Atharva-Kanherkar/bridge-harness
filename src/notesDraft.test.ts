import { describe, expect, it } from "vitest";
import { noteDisplayTitle, plainPreview, quoteText, rebaseDraft } from "./notesDraft";

describe("rebaseDraft", () => {
  const sent = "alpha\nbeta\ngamma\n";

  it("adopts the merged text when nothing was typed during the save", () => {
    expect(rebaseDraft(sent, sent, `${sent}agent line\n`)).toEqual({ text: `${sent}agent line\n`, overlap: false });
  });

  it("keeps the draft when the save merged nothing", () => {
    expect(rebaseDraft(sent, `${sent}mine\n`, sent)).toEqual({ text: `${sent}mine\n`, overlap: false });
  });

  it("replays typing before a concurrent change", () => {
    const draft = "alpha!\nbeta\ngamma\n";
    const merged = `${sent}agent line\n`;
    expect(rebaseDraft(sent, draft, merged)).toEqual({ text: "alpha!\nbeta\ngamma\nagent line\n", overlap: false });
  });

  it("replays typing after a concurrent change", () => {
    const draft = "alpha\nbeta\ngamma\nmine\n";
    const merged = "agent first\nalpha\nbeta\ngamma\n";
    expect(rebaseDraft(sent, draft, merged)).toEqual({ text: "agent first\nalpha\nbeta\ngamma\nmine\n", overlap: false });
  });

  it("merges typing that only touches a concurrent insertion", () => {
    expect(rebaseDraft(sent, "alpha\nBETA\ngamma\n", "alpha\nbeta (agent)\ngamma\n")).toEqual({ text: "alpha\nBETA (agent)\ngamma\n", overlap: false });
  });

  it("keeps separate concurrent edits on both sides", () => {
    // The reviewer's case: the person changes the first and last lines while
    // an agent changes only the middle one.
    const base = "alpha!\nbeta\ngamma\n";
    expect(rebaseDraft(base, "ALPHA!\nbeta\nGAMMA\n", "alpha!\nBETA\ngamma\n")).toEqual({ text: "ALPHA!\nBETA\nGAMMA\n", overlap: false });
    // Two edits inside one line, around the agent's.
    expect(rebaseDraft("one two three\n", "ONE two THREE\n", "one TWO three\n")).toEqual({ text: "ONE TWO THREE\n", overlap: false });
    // Agent edits on both sides of the person's.
    expect(rebaseDraft("a\nb\nc\nd\ne\n", "a\nb\nC!\nd\ne\n", "A\nb\nc\nd\nE\n")).toEqual({ text: "A\nb\nC!\nd\nE\n", overlap: false });
  });

  it("keeps every edit when the two sides change different lines", () => {
    // Seeded so a failure reproduces: the person edits even lines, the agent
    // odd ones, including neighbours and inserted lines.
    let seed = 7;
    const random = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
    for (let round = 0; round < 300; round += 1) {
      const base = Array.from({ length: 2 + Math.floor(random() * 10) }, (_, index) => `line ${index} ${"x".repeat(Math.floor(random() * 4))}\n`);
      const edit = (parity: number, mark: string) => base.map((line, index) => index % 2 === parity && random() < 0.6 ? (random() < 0.3 ? `${line}${mark} added\n` : `${mark} ${line}`) : line);
      const mine = edit(0, "M");
      const theirs = edit(1, "T");
      const expected = base.map((_, index) => (index % 2 === 0 ? mine : theirs)[index]).join("");
      expect(rebaseDraft(base.join(""), mine.join(""), theirs.join("")), `round ${round}`).toEqual({ text: expected, overlap: false });
    }
  });

  it("keeps the agent's other edits when one span conflicts", () => {
    expect(rebaseDraft("a\nb\nc\n", "a\nmine\nc\n", "A\ntheirs\nc\n")).toEqual({ text: "A\nmine\nc\n", overlap: true });
  });

  it("keeps the person's draft when both changed the same span", () => {
    const draft = "alpha\nBETA\ngamma\n";
    expect(rebaseDraft(sent, draft, "alpha\nbXta\ngamma\n")).toEqual({ text: draft, overlap: true });
  });
});

describe("noteDisplayTitle", () => {
  it("prefers the title, then the first non-empty line, then Untitled", () => {
    expect(noteDisplayTitle({ title: " Launch ", preview: "x" })).toBe("Launch");
    expect(noteDisplayTitle({ title: null, preview: "\n# Heading here\nmore" })).toBe("Heading here");
    expect(noteDisplayTitle({ title: "", preview: "   \n" })).toBe("Untitled");
    expect(noteDisplayTitle({ title: null, preview: "x".repeat(120) })).toHaveLength(80);
  });
});

describe("plainPreview and quoteText", () => {
  it("reads Markdown as one plain line", () => {
    expect(plainPreview("# Sync\n\nThree worth a look:\n\n1. **Yjs**: CRDTs, see [docs](https://x.y)\n- `code` and _soft_ ~~gone~~")).toBe("Sync Three worth a look: Yjs: CRDTs, see docs code and soft gone");
    expect(plainPreview("plain words, kept")).toBe("plain words, kept");
  });
  it("drops the list marker a quote was typed with", () => {
    expect(quoteText("- add a skip button")).toBe("add a skip button");
    expect(quoteText("2. second")).toBe("second");
    expect(quoteText("need to - keep dashes")).toBe("need to - keep dashes");
  });
});
