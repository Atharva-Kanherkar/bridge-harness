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
