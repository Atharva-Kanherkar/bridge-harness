import { describe, expect, it } from "vitest";
import { noteLink, parseNoteLink } from "./noteLinks";

const ID = "01a1bc34-8f0f-7013-a72c-51e46d404d3c";

describe("parseNoteLink", () => {
  it("parses a page link", () => {
    expect(parseNoteLink(`bridge://p/${ID}`)).toBe(ID);
    expect(parseNoteLink(` BRIDGE://p/${ID} `)).toBe(ID);
    expect(parseNoteLink("bridge://p/note-000b")).toBe("note-000b");
    expect(parseNoteLink(noteLink(ID))).toBe(ID);
  });

  it("rejects anything that is not exactly a page link", () => {
    for (const url of [
      `bridge://p/${ID}/more`, `bridge://p/${ID}?x=1`, `bridge://p/${ID}#h`, "bridge://p/", "bridge://p/a.b", "bridge://p/a b", `bridge://p/${"x".repeat(65)}`,
      `bridge://x/${ID}`, `https://example.com/p/${ID}`, "javascript:alert(1)", "",
    ]) expect(parseNoteLink(url), url).toBeUndefined();
  });
});
