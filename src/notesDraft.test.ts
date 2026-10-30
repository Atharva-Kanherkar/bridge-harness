import { describe, expect, it } from "vitest";
import { noteDisplayTitle, rebaseDraft } from "./notesDraft";

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
