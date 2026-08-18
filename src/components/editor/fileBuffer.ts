import { bridgeApi } from "../../api";
import { errorMessage } from "../../errors";

/** How a buffer stands relative to the bytes on disk. */
export type FileState = "clean" | "dirty" | "saving" | "conflict" | "error";

/** One workspace file held open for editing. */
export interface FileBuffer {
  path: string;
  /** Hash of the bytes on disk that `saved` came from — the write token. */
  baseSha: string;
  /** Content as of the last read or successful save: the dirty baseline. */
  saved: string;
  binary: boolean;
  tooLarge: boolean;
  sizeBytes: number;
  /** Bumped to re-seed the editor: a fresh open, or a reload from disk. */
  seed: number;
  state: FileState;
  message?: string;
}

/** Nothing to type into: the editor shows a notice instead. */
export const isReadOnly = (buffer: FileBuffer): boolean => buffer.binary || buffer.tooLarge;

export const isDirty = (buffer: FileBuffer): boolean =>
  buffer.state === "dirty" || buffer.state === "conflict";

/** Read a file into a fresh buffer. Failures become an `error` buffer, so a
 *  caller never has to special-case "the tab exists but the read threw". */
export async function loadBuffer(workspaceId: string, path: string, seed = 0): Promise<FileBuffer> {
  try {
    const file = await bridgeApi.readWorkspaceFile(workspaceId, path);
    return {
      path,
      baseSha: file.sha256,
      saved: file.content,
      binary: file.binary,
      tooLarge: file.tooLarge,
      sizeBytes: file.sizeBytes,
      seed,
      state: "clean",
    };
  } catch (error) {
    return {
      path, baseSha: "", saved: "", binary: false, tooLarge: false, sizeBytes: 0,
      seed, state: "error", message: errorMessage(error),
    };
  }
}

/**
 * Save `content` over `buffer` and return the buffer that results.
 *
 * The write carries the hash the buffer was read at, so a file an agent
 * touched since is refused rather than clobbered. `force` re-reads first and
 * writes against whatever is on disk now — still a real hash, so it races
 * nothing; it just means "I have seen the change and I want mine".
 */
export async function saveBuffer(workspaceId: string, buffer: FileBuffer, content: string, force = false): Promise<FileBuffer> {
  try {
    const base = force ? (await bridgeApi.readWorkspaceFile(workspaceId, buffer.path)).sha256 : buffer.baseSha;
    const { sha256 } = await bridgeApi.writeWorkspaceFile(workspaceId, buffer.path, content, base);
    return { ...buffer, baseSha: sha256, saved: content, state: "clean", message: undefined };
  } catch (error) {
    const message = errorMessage(error);
    // The backend phrases the lost-update refusal this way; anything else is
    // a genuine failure the user cannot resolve by choosing a side.
    const conflict = message.includes("changed on disk") || message.includes("no longer exists");
    return { ...buffer, state: conflict ? "conflict" : "error", message };
  }
}

/**
 * The state a buffer should take after an edit — or `null` when nothing
 * visible changed, which is the signal to skip the re-render entirely. Typing
 * inside an already-dirty file must not re-render the surrounding UI.
 */
export function stateAfterEdit(buffer: FileBuffer, value: string): FileState | null {
  if (buffer.state !== "clean" && buffer.state !== "dirty") return null;
  const next: FileState = value === buffer.saved ? "clean" : "dirty";
  return next === buffer.state ? null : next;
}

/** What the status line says about a buffer. */
export function statusLabel(buffer: FileBuffer): string {
  if (buffer.tooLarge) return "Too large to edit";
  if (buffer.binary) return "Binary — read only";
  switch (buffer.state) {
    case "saving": return "Saving…";
    case "conflict": return buffer.message ?? "Changed on disk";
    case "error": return buffer.message ?? "Could not save";
    case "dirty": return "Unsaved changes";
    default: return "Saved";
  }
}
