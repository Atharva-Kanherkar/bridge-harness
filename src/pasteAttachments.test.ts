import { describe, expect, it } from "vitest";
import { imageFilesFromClipboard, isPasteTooLarge, MAX_PASTE_BYTES, mediaTypeOf } from "./pasteAttachments";

const item = (kind: string, type: string, file: { type: string; size?: number } | null) => ({
  kind,
  type,
  getAsFile: () => file,
});

describe("imageFilesFromClipboard", () => {
  it("maps image file items to files, in clipboard order", () => {
    const files = imageFilesFromClipboard([
      item("string", "text/plain", null),
      item("file", "image/png", { type: "image/png", size: 10 }),
      item("file", "image/jpeg", { type: "image/jpeg", size: 20 }),
    ]);
    expect(files.map(file => file.type)).toEqual(["image/png", "image/jpeg"]);
  });

  it("ignores non-image files and text flavors", () => {
    const files = imageFilesFromClipboard([
      item("file", "application/pdf", { type: "application/pdf", size: 5 }),
      item("string", "image/png", null),
      item("file", "image/webp", null),
    ]);
    expect(files).toEqual([]);
  });

  it("keeps a lone screenshot so a mixed paste still attaches it", () => {
    const files = imageFilesFromClipboard([
      item("string", "text/html", null),
      item("file", "image/png", { type: "image/png", size: 12 }),
    ]);
    expect(files).toHaveLength(1);
  });
});

describe("mediaTypeOf", () => {
  it("passes the file's media type through", () => {
    expect(mediaTypeOf({ type: "image/png" })).toBe("image/png");
  });

  it("never hands the wire an empty media type", () => {
    expect(mediaTypeOf({ type: "" })).toBe("application/octet-stream");
  });
});

describe("isPasteTooLarge", () => {
  it("accepts a normal screenshot and refuses something absurd", () => {
    expect(isPasteTooLarge({ type: "image/png", size: 1024 * 1024 })).toBe(false);
    expect(isPasteTooLarge({ type: "image/png", size: MAX_PASTE_BYTES + 1 })).toBe(true);
  });

  it("does not refuse a file whose size is unknown", () => {
    expect(isPasteTooLarge({ type: "image/png" })).toBe(false);
  });
});
