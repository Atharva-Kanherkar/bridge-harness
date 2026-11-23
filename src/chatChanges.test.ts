import { describe, expect, it } from "vitest";
import { reviewBadge } from "./chatChanges";
import type { SessionChangesResult, WorkspaceFileChange } from "./types";

const folder = { dirtyFiles: 1817, additions: 291222, deletions: 37 };

const file = (path: string): WorkspaceFileChange => ({
  path, previousPath: null, changeKind: "modified", additions: 1, deletions: 0, patch: "",
  patchTruncated: false, binary: false, importance: "low", labels: [], lowSignal: false,
});

const chat = (over: Partial<SessionChangesResult> = {}): SessionChangesResult => ({
  available: true, unavailableReason: null, turns: 2, since: "2026-11-21T10:00:00Z", additions: 12, deletions: 3,
  changes: { baseCommit: null, repositoryState: "normal", files: [file("a.ts"), file("b.ts")], totalFiles: 2, filesTruncated: false },
  ...over,
});

describe("reviewBadge", () => {
  it("counts the chat's own files, not the folder's", () => {
    expect(reviewBadge(chat(), true, folder)).toEqual({ scope: "chat", files: 2, additions: 12, deletions: 3 });
  });

  it("uses the chat's full file total when its list was cut", () => {
    const cut = chat({ changes: { ...chat().changes!, totalFiles: 340, filesTruncated: true } });
    expect(reviewBadge(cut, true, folder)?.files).toBe(340);
  });

  it("shows nothing when the chat changed nothing, however dirty the folder is", () => {
    const quiet = chat({ additions: 0, deletions: 0, changes: { ...chat().changes!, files: [], totalFiles: 0 } });
    expect(reviewBadge(quiet, true, folder)).toBeNull();
  });

  it("waits for the chat's first answer instead of flashing the folder's number", () => {
    expect(reviewBadge(undefined, false, folder)).toBeNull();
  });

  it("falls back to the folder, labelled as such, when the chat cannot be measured", () => {
    const fresh = chat({ available: false, unavailableReason: "no_turns", changes: null, turns: 0 });
    expect(reviewBadge(fresh, true, folder)).toEqual({ scope: "folder", files: 1817, additions: 291222, deletions: 37 });
    // An older daemon without the method: the poll failed, so nothing came back.
    expect(reviewBadge(undefined, true, folder)?.scope).toBe("folder");
    expect(reviewBadge(undefined, true, { dirtyFiles: 0, additions: 0, deletions: 0 })).toBeNull();
  });
});
