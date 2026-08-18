import { beforeEach, describe, expect, it, vi } from "vitest";
import { isDirty, isReadOnly, loadBuffer, saveBuffer, stateAfterEdit, statusLabel, type FileBuffer } from "./fileBuffer";
import { bridgeApi } from "../../api";

vi.mock("../../api", () => ({
  bridgeApi: { readWorkspaceFile: vi.fn(), writeWorkspaceFile: vi.fn() },
}));

const read = vi.mocked(bridgeApi.readWorkspaceFile);
const write = vi.mocked(bridgeApi.writeWorkspaceFile);

const buffer = (over: Partial<FileBuffer> = {}): FileBuffer => ({
  path: "src/App.tsx", baseSha: "aaa", saved: "one", binary: false, tooLarge: false,
  sizeBytes: 3, seed: 0, state: "clean", ...over,
});

beforeEach(() => vi.resetAllMocks());

describe("loadBuffer", () => {
  it("reads a file into a clean buffer", async () => {
    read.mockResolvedValue({ path: "a.ts", content: "x", sha256: "sha", tooLarge: false, binary: false, sizeBytes: 1 });
    expect(await loadBuffer("w", "a.ts")).toMatchObject({ path: "a.ts", saved: "x", baseSha: "sha", state: "clean" });
  });

  it("turns a failed read into an error buffer instead of throwing", async () => {
    read.mockRejectedValue(new Error("a.ts is not a file"));
    const result = await loadBuffer("w", "a.ts");
    expect(result.state).toBe("error");
    expect(result.message).toBe("a.ts is not a file");
  });

  it("carries the seed through, so a reload re-seeds the editor", async () => {
    read.mockResolvedValue({ path: "a.ts", content: "x", sha256: "sha", tooLarge: false, binary: false, sizeBytes: 1 });
    expect((await loadBuffer("w", "a.ts", 3)).seed).toBe(3);
  });
});

describe("saveBuffer", () => {
  it("writes with the hash the buffer was read at and adopts the new one", async () => {
    write.mockResolvedValue({ sha256: "bbb" });
    const result = await saveBuffer("w", buffer(), "two");
    expect(write).toHaveBeenCalledWith("w", "src/App.tsx", "two", "aaa");
    expect(result).toMatchObject({ state: "clean", baseSha: "bbb", saved: "two" });
  });

  it("reports a conflict rather than clobbering when disk has moved", async () => {
    write.mockRejectedValue(new Error("src/App.tsx changed on disk since it was opened"));
    const result = await saveBuffer("w", buffer({ state: "dirty" }), "two");
    expect(result.state).toBe("conflict");
    // The baseline is untouched, so "Reload" and "Overwrite" both stay available.
    expect(result.saved).toBe("one");
    expect(result.baseSha).toBe("aaa");
  });

  it("treats a vanished file as a conflict too", async () => {
    write.mockRejectedValue(new Error("src/App.tsx no longer exists on disk"));
    expect((await saveBuffer("w", buffer(), "two")).state).toBe("conflict");
  });

  it("keeps an ordinary failure out of the conflict path", async () => {
    write.mockRejectedValue(new Error("Permission denied"));
    expect((await saveBuffer("w", buffer(), "two")).state).toBe("error");
  });

  it("re-reads before a forced overwrite so the write still carries a real hash", async () => {
    read.mockResolvedValue({ path: "src/App.tsx", content: "agent", sha256: "ccc", tooLarge: false, binary: false, sizeBytes: 5 });
    write.mockResolvedValue({ sha256: "ddd" });
    const result = await saveBuffer("w", buffer({ state: "conflict" }), "mine", true);
    expect(write).toHaveBeenCalledWith("w", "src/App.tsx", "mine", "ccc");
    expect(result).toMatchObject({ state: "clean", saved: "mine", baseSha: "ddd" });
  });
});

describe("stateAfterEdit", () => {
  it("returns null when nothing visible changed, so typing skips the re-render", () => {
    expect(stateAfterEdit(buffer({ state: "dirty" }), "two")).toBeNull();
    expect(stateAfterEdit(buffer({ state: "clean" }), "one")).toBeNull();
  });

  it("flips clean to dirty and back", () => {
    expect(stateAfterEdit(buffer({ state: "clean" }), "two")).toBe("dirty");
    expect(stateAfterEdit(buffer({ state: "dirty" }), "one")).toBe("clean");
  });

  it("leaves a saving or conflicted buffer alone", () => {
    expect(stateAfterEdit(buffer({ state: "saving" }), "two")).toBeNull();
    expect(stateAfterEdit(buffer({ state: "conflict" }), "two")).toBeNull();
  });
});

describe("buffer predicates", () => {
  it("treats binary and oversized files as read only", () => {
    expect(isReadOnly(buffer({ binary: true }))).toBe(true);
    expect(isReadOnly(buffer({ tooLarge: true }))).toBe(true);
    expect(isReadOnly(buffer())).toBe(false);
  });

  it("counts conflicted buffers as unsaved work", () => {
    expect(isDirty(buffer({ state: "conflict" }))).toBe(true);
    expect(isDirty(buffer({ state: "dirty" }))).toBe(true);
    expect(isDirty(buffer({ state: "clean" }))).toBe(false);
  });

  it("says what state a buffer is in", () => {
    expect(statusLabel(buffer())).toBe("Saved");
    expect(statusLabel(buffer({ binary: true }))).toBe("Binary — read only");
    expect(statusLabel(buffer({ state: "conflict", message: "x changed on disk" }))).toBe("x changed on disk");
  });
});
