// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { dataUriBytes, fittedSize, imageFilesFromClipboard, MAX_IMAGE_BYTES, MAX_IMAGE_EDGE, mediaTypeOf, prepareImage, shrinkToFit } from "./pasteAttachments";

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

describe("dataUriBytes", () => {
  it("counts decoded bytes, not base64 characters", () => {
    expect(dataUriBytes(`data:image/png;base64,${btoa("abc")}`)).toBe(3);
    expect(dataUriBytes(`data:image/png;base64,${btoa("abcd")}`)).toBe(4);
    expect(dataUriBytes(`data:image/png;base64,${btoa("abcde")}`)).toBe(5);
  });
});

describe("fittedSize", () => {
  it("leaves an image within the edge alone", () => {
    expect(fittedSize(1200, 800)).toEqual({ width: 1200, height: 800 });
  });

  it("scales the longest edge down to the cap, keeping the aspect ratio", () => {
    expect(fittedSize(6048, 3928)).toEqual({ width: MAX_IMAGE_EDGE, height: 1299 });
    expect(fittedSize(1000, 8000)).toEqual({ width: 250, height: MAX_IMAGE_EDGE });
  });
});

/** A fake encoder whose output size is proportional to pixels, scaled by format. */
function fakeEncoder(bytesPerPixel: Record<string, number>) {
  const calls: Array<{ width: number; height: number; mediaType: string; quality?: number }> = [];
  const encode = async (width: number, height: number, mediaType: string, quality?: number) => {
    calls.push({ width, height, mediaType, quality });
    const bytes = Math.round(width * height * bytesPerPixel[mediaType] * (quality ?? 1));
    return `data:${mediaType};base64,${"A".repeat(Math.ceil(bytes / 3) * 4)}`;
  };
  return { encode, calls };
}

describe("shrinkToFit", () => {
  it("keeps a resized screenshot as PNG when it fits", async () => {
    const { encode, calls } = fakeEncoder({ "image/png": 0.5, "image/jpeg": 0.2 });
    const result = await shrinkToFit(6048, 3928, "image/png", encode);
    expect(result.mediaType).toBe("image/png");
    expect(calls).toEqual([{ width: 2000, height: 1299, mediaType: "image/png", quality: undefined }]);
  });

  it("falls back to JPEG when the PNG is still over budget", async () => {
    const { encode } = fakeEncoder({ "image/png": 4, "image/jpeg": 0.5 });
    const result = await shrinkToFit(4000, 3000, "image/png", encode);
    expect(result.mediaType).toBe("image/jpeg");
    expect(dataUriBytes(result.dataUri)).toBeLessThanOrEqual(MAX_IMAGE_BYTES);
  });

  it("keeps shrinking until even a pathological image fits", async () => {
    const { encode, calls } = fakeEncoder({ "image/png": 20, "image/jpeg": 10 });
    const result = await shrinkToFit(8000, 8000, "image/jpeg", encode);
    expect(dataUriBytes(result.dataUri)).toBeLessThanOrEqual(MAX_IMAGE_BYTES);
    expect(calls.every(call => call.mediaType === "image/jpeg")).toBe(true);
    expect(Math.max(calls.at(-1)!.width, calls.at(-1)!.height)).toBeLessThan(MAX_IMAGE_EDGE);
  });
});

describe("prepareImage", () => {
  it("passes the original bytes through when nothing can decode them", async () => {
    const file = new Blob(["png-bytes"], { type: "image/png" });
    const result = await prepareImage(file);
    expect(result.mediaType).toBe("image/png");
    expect(atob(result.dataUri.split(",")[1])).toBe("png-bytes");
  });
});
