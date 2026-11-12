/**
 * Clipboard → attachment extraction for the composer.
 *
 * Extracted from the paste handler so the interesting part — which clipboard
 * flavors become attachments — is unit-testable without synthesizing
 * ClipboardEvents. The handler owns the synchronous preventDefault; this
 * helper is only the mapping.
 */

export interface ComposerAttachment {
  id: string;
  mediaType: string;
  /** Full `data:<mime>;base64,…` URI, ready to render and to send. */
  dataUri: string;
}

interface PasteFile {
  type: string;
  size?: number;
}

/** Image files hiding in clipboard items, in clipboard order. */
export function imageFilesFromClipboard(
  items: ArrayLike<{ kind: string; type: string; getAsFile: () => PasteFile | null }>,
): PasteFile[] {
  const files: PasteFile[] = [];
  for (let index = 0; index < items.length; index += 1) {
    const item = items[index];
    if (item.kind !== "file" || !item.type.startsWith("image/")) continue;
    const file = item.getAsFile();
    if (file) files.push(file);
  }
  return files;
}

/** A file's media type, as Bridge's wire type wants it. */
export function mediaTypeOf(file: PasteFile): string {
  return file.type || "application/octet-stream";
}

/**
 * Longest edge an attached image keeps. Claude refuses edges over 2000 px once
 * a request carries many images, and every provider downscales past roughly
 * that anyway, so nothing a model would see is lost.
 */
export const MAX_IMAGE_EDGE = 2000;

/**
 * Decoded bytes an attached image may carry. Base64 inflates this by 4/3,
 * which keeps it under Claude's 5 MB per-image cap with headroom.
 */
export const MAX_IMAGE_BYTES = 3_500_000;

/** Media types every provider accepts inline; anything else is re-encoded. */
const SENDABLE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

/** Decoded byte count of a base64 data URI's payload. */
export function dataUriBytes(dataUri: string): number {
  const payload = dataUri.slice(dataUri.indexOf(",") + 1);
  const padding = payload.endsWith("==") ? 2 : payload.endsWith("=") ? 1 : 0;
  return Math.floor(payload.length * 3 / 4) - padding;
}

/** `width`×`height` scaled down so the longest edge is at most `maxEdge`. */
export function fittedSize(width: number, height: number, maxEdge = MAX_IMAGE_EDGE): { width: number; height: number } {
  const longest = Math.max(width, height);
  if (longest <= maxEdge) return { width, height };
  const scale = maxEdge / longest;
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** Draws the image at a size and returns it as a data URI of `mediaType`. */
export type ImageEncoder = (width: number, height: number, mediaType: string, quality?: number) => Promise<string>;

/**
 * Re-encode an image until it fits the edge and byte budgets: PNG first when
 * the source is PNG, so a resized screenshot keeps crisp text; then JPEG at
 * falling quality, then smaller, so any paste ends up sendable.
 */
export async function shrinkToFit(
  width: number,
  height: number,
  mediaType: string,
  encode: ImageEncoder,
): Promise<Pick<ComposerAttachment, "mediaType" | "dataUri">> {
  let size = fittedSize(width, height);
  if (mediaType === "image/png") {
    const png = await encode(size.width, size.height, "image/png");
    if (dataUriBytes(png) <= MAX_IMAGE_BYTES) return { mediaType: "image/png", dataUri: png };
  }
  for (;;) {
    let jpeg = "";
    for (const quality of [0.9, 0.8, 0.7]) {
      jpeg = await encode(size.width, size.height, "image/jpeg", quality);
      if (dataUriBytes(jpeg) <= MAX_IMAGE_BYTES) return { mediaType: "image/jpeg", dataUri: jpeg };
    }
    if (Math.max(size.width, size.height) <= 256) return { mediaType: "image/jpeg", dataUri: jpeg };
    size = fittedSize(size.width, size.height, Math.round(Math.max(size.width, size.height) * 0.75));
  }
}

function canvasEncoder(bitmap: ImageBitmap): ImageEncoder {
  return async (width, height, mediaType, quality) => {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not resize the pasted image");
    // JPEG has no alpha; transparent pixels would otherwise turn black.
    if (mediaType === "image/jpeg") {
      context.fillStyle = "#fff";
      context.fillRect(0, 0, width, height);
    }
    context.imageSmoothingQuality = "high";
    context.drawImage(bitmap, 0, 0, width, height);
    return canvas.toDataURL(mediaType, quality);
  };
}

/**
 * A pasted image as an attachment every provider accepts: kept as-is when it
 * already fits, otherwise resized and re-encoded. Without a decoder (tests,
 * an undecodable format) the original bytes pass through unchanged.
 */
export async function prepareImage(file: PasteFile): Promise<Pick<ComposerAttachment, "mediaType" | "dataUri">> {
  const mediaType = mediaTypeOf(file);
  const dataUri = await readAsDataUri(file);
  if (typeof createImageBitmap !== "function") return { mediaType, dataUri };
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file as Blob);
  } catch {
    return { mediaType, dataUri };
  }
  try {
    const fits = SENDABLE_TYPES.has(mediaType)
      && dataUriBytes(dataUri) <= MAX_IMAGE_BYTES
      && Math.max(bitmap.width, bitmap.height) <= MAX_IMAGE_EDGE;
    if (fits) return { mediaType, dataUri };
    return await shrinkToFit(bitmap.width, bitmap.height, mediaType, canvasEncoder(bitmap));
  } finally {
    bitmap.close();
  }
}

/** `File` → data URI, resolved once the bytes are in hand. */
export function readAsDataUri(file: PasteFile): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("Could not read the pasted image"));
    reader.readAsDataURL(file as File);
  });
}
