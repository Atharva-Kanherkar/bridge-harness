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
 * One `ComposerAttachment` per clipboard image. Rejected before decode:
 * screenshots are megabytes, and a silent multi-second paste feels broken —
 * the caller surfaces an inline message instead.
 */
export const MAX_PASTE_BYTES = 8 * 1024 * 1024;

export function isPasteTooLarge(file: PasteFile): boolean {
  return typeof file.size === "number" && file.size > MAX_PASTE_BYTES;
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
