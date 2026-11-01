// Images on a page: uploaded in parts (a request frame is capped at 1 MiB),
// stored by the core under `assets/<hash>.<ext>` beside the page files, and
// read back once per session.

import { bridgeApi } from "../../../api";

export const ASSET_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml"];
export const MAX_ASSET_BYTES = 20 * 1024 * 1024;
/** Raw bytes per part: a multiple of 3, so every part is whole base64, and
 *  about 680 KiB on the wire. */
const PART_BYTES = 510 * 1024;

/** Why `file` cannot go on a page, or null when it can. */
export function assetRefusal(file: Pick<File, "type" | "size" | "name">): string | null {
  if (!ASSET_TYPES.includes(file.type)) return `${file.name || "That file"} is not an image Bridge can show (PNG, JPEG, GIF, WebP or SVG).`;
  if (file.size > MAX_ASSET_BYTES) return `${file.name || "That image"} is over 20 MB.`;
  return null;
}

const base64 = (bytes: Uint8Array) => {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  return btoa(binary);
};

const readBytes = (file: Blob): Promise<ArrayBuffer> => typeof file.arrayBuffer === "function"
  ? file.arrayBuffer()
  : new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as ArrayBuffer);
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(file);
  });

/** Store `file`; resolves to its page path, `assets/<hash>.<ext>`. */
export async function uploadAsset(file: File): Promise<string> {
  const refusal = assetRefusal(file);
  if (refusal) throw new Error(refusal);
  const bytes = new Uint8Array(await readBytes(file));
  let uploadId: string | null = null;
  for (let offset = 0; offset < bytes.length || uploadId === null; offset += PART_BYTES) {
    const part = await bridgeApi.putNoteAssetPart({ uploadId, data: base64(bytes.subarray(offset, offset + PART_BYTES)) });
    uploadId = part.uploadId;
  }
  const asset = await bridgeApi.finishNoteAsset({ uploadId, mime: file.type });
  // The bytes are here already; no need to read them back.
  if (typeof URL.createObjectURL === "function") cache.set(asset.path, Promise.resolve(URL.createObjectURL(file)));
  return asset.path;
}

const cache = new Map<string, Promise<string>>();

/** A URL the page can show for `src`: stored assets are read from the core,
 *  anything else is used as written. */
export function assetUrl(src: string): Promise<string> {
  if (!isAssetPath(src)) return Promise.resolve(src);
  let url = cache.get(src);
  if (!url) {
    url = bridgeApi.readNoteAsset(src).then(asset => `data:${asset.mime};base64,${asset.data}`);
    url.catch(() => cache.delete(src));
    cache.set(src, url);
  }
  return url;
}

export const isAssetPath = (src: string) => /^assets\/[0-9a-f]{16}\.[a-z0-9]+$/.test(src);
