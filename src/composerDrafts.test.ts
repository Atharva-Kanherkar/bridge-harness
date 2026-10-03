import { describe, expect, it } from "vitest";
import { ComposerDrafts, EMPTY_DRAFT, isEmptyDraft, mergeFailedAttachments, mergeFailedSend, mergeFailedText, withoutSent, withoutSentAttachments, withoutSentText } from "./composerDrafts";
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

describe("withoutSentText", () => {
  it("empties a composer that holds exactly the resent words", () => {
    expect(withoutSentText("prev", "prev")).toBe("");
    expect(withoutSentText("  prev \n", "prev")).toBe("");
  });

  it("removes the restored copy ahead of newer typing", () => {
    expect(withoutSentText(mergeFailedText("next", "prev"), "prev")).toBe("next");
  });

  it("finds the restored copy whatever whitespace surrounded the failed words", () => {
    expect(withoutSentText(mergeFailedText("next", "  prev \n"), "prev")).toBe("next");
    expect(withoutSentText(mergeFailedText("next", "prev"), " prev ")).toBe("next");
  });

  it("keeps what was typed after the sent words", () => {
    expect(withoutSentText("$claude review this and the tests", "$claude review this")).toBe("and the tests");
    expect(withoutSentText("  /btw why?\nmore", "/btw why?")).toBe("more");
  });

  it("leaves unrelated typing alone", () => {
    expect(withoutSentText("something else", "prev")).toBe("something else");
    expect(withoutSentText("previous thoughts", "prev")).toBe("previous thoughts");
  });

  it("never empties the composer for an image-only resend", () => {
    expect(withoutSentText("typing", "")).toBe("typing");
  });
});

describe("withoutSentAttachments", () => {
  it("removes only the resent images", () => {
    expect(withoutSentAttachments([image("a"), image("b")], [image("a")]).map(item => item.id)).toEqual(["b"]);
  });
});

describe("withoutSent", () => {
  it("takes the sent words and images and keeps an image pasted since", () => {
    const left = withoutSent({ text: "$claude review this", attachments: [image("late")] }, { text: "$claude review this", attachments: [] });
    expect(left.text).toBe("");
    expect(left.attachments.map(item => item.id)).toEqual(["late"]);
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
