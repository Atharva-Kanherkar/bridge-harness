import { describe, expect, it } from "vitest";
import { ComposerDrafts, EMPTY_DRAFT, isEmptyDraft, mergeFailedAttachments, mergeFailedSend, mergeFailedText, withoutResentAttachments, withoutResentText } from "./composerDrafts";
import type { ComposerAttachment } from "./pasteAttachments";

const image = (id: string): ComposerAttachment => ({ id, mediaType: "image/png", dataUri: `data:image/png;base64,${id}` });

describe("mergeFailedText", () => {
  it("returns the failed words into an empty composer", () => {
    expect(mergeFailedText("", "prev")).toBe("prev");
  });

  it("treats a whitespace-only composer as empty", () => {
    expect(mergeFailedText("  \n ", "prev")).toBe("prev");
  });

  it("puts the failed words first, a blank line, then the newer typing", () => {
    expect(mergeFailedText("next", "prev")).toBe("prev\n\nnext");
  });

  it("leaves the composer alone when the failed send had no words", () => {
    expect(mergeFailedText("next", "")).toBe("next");
    expect(mergeFailedText("next", "   ")).toBe("next");
  });
});

describe("mergeFailedAttachments", () => {
  it("puts the failed images first and does not add one twice", () => {
    const merged = mergeFailedAttachments([image("b"), image("a")], [image("a"), image("c")]);
    expect(merged.map(item => item.id)).toEqual(["a", "c", "b"]);
  });

  it("returns the failed images into an empty composer", () => {
    expect(mergeFailedAttachments([], [image("a")]).map(item => item.id)).toEqual(["a"]);
  });
});

describe("mergeFailedSend", () => {
  it("merges words and images together", () => {
    const merged = mergeFailedSend({ text: "next", attachments: [image("b")] }, { text: "prev", attachments: [image("a")] });
    expect(merged.text).toBe("prev\n\nnext");
    expect(merged.attachments.map(item => item.id)).toEqual(["a", "b"]);
  });
});

describe("withoutResentText", () => {
  it("empties a composer that holds exactly the resent words", () => {
    expect(withoutResentText("prev", "prev")).toBe("");
    expect(withoutResentText("  prev \n", "prev")).toBe("");
  });

  it("removes the restored copy ahead of newer typing", () => {
    expect(withoutResentText(mergeFailedText("next", "prev"), "prev")).toBe("next");
  });

  it("finds the restored copy whatever whitespace surrounded the failed words", () => {
    expect(withoutResentText(mergeFailedText("next", "  prev \n"), "prev")).toBe("next");
    expect(withoutResentText(mergeFailedText("next", "prev"), " prev ")).toBe("next");
  });

  it("leaves unrelated typing alone", () => {
    expect(withoutResentText("something else", "prev")).toBe("something else");
    expect(withoutResentText("previous thoughts", "prev")).toBe("previous thoughts");
  });

  it("never empties the composer for an image-only resend", () => {
    expect(withoutResentText("typing", "")).toBe("typing");
  });
});

describe("withoutResentAttachments", () => {
  it("removes only the resent images", () => {
    expect(withoutResentAttachments([image("a"), image("b")], [image("a")]).map(item => item.id)).toEqual(["b"]);
  });
});

describe("ComposerDrafts", () => {
  it("hands a draft back once and then forgets it", () => {
    const drafts = new ComposerDrafts();
    drafts.put("a", { text: "x", attachments: [image("1")] });
    expect(drafts.peek("a").text).toBe("x");
    expect(drafts.take("a")).toEqual({ text: "x", attachments: [image("1")] });
    expect(drafts.take("a")).toBe(EMPTY_DRAFT);
    expect(drafts.size).toBe(0);
  });

  it("does not store an empty draft, and an empty put clears a stored one", () => {
    const drafts = new ComposerDrafts();
    drafts.put("a", { text: "   ", attachments: [] });
    expect(drafts.size).toBe(0);
    drafts.put("a", { text: "x", attachments: [] });
    drafts.put("a", EMPTY_DRAFT);
    expect(drafts.size).toBe(0);
  });

  it("keeps an image-only draft", () => {
    const drafts = new ComposerDrafts();
    drafts.put("a", { text: "", attachments: [image("1")] });
    expect(drafts.take("a").attachments).toHaveLength(1);
  });

  it("drops drafts of chats that no longer exist", () => {
    const drafts = new ComposerDrafts();
    drafts.put("a", { text: "x", attachments: [] });
    drafts.put("b", { text: "y", attachments: [] });
    drafts.retain(new Set(["b"]));
    expect(drafts.peek("a")).toBe(EMPTY_DRAFT);
    expect(drafts.peek("b").text).toBe("y");
  });

  it("reports emptiness by words and images", () => {
    expect(isEmptyDraft(EMPTY_DRAFT)).toBe(true);
    expect(isEmptyDraft({ text: " ", attachments: [] })).toBe(true);
    expect(isEmptyDraft({ text: "", attachments: [image("1")] })).toBe(false);
  });
});
