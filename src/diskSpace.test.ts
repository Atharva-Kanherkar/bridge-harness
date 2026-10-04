import { expect, it } from "vitest";
import { cleanupTotal, diskBytes, displayPath, parseStoragePlan, splitSnapshot, withSnapshot } from "./diskSpace";
import type { DiskSuggestion } from "./types";

const suggestion = (path: string, sizeBytes: number, safety = "safe"): DiskSuggestion =>
  ({ id: path, label: path, group: "caches", description: "", path, sizeBytes, measuring: false, safety });

it("formats sizes in the decimal units Finder uses", () => {
  expect(diskBytes(19_400_000_000)).toBe("19.4 GB");
  expect(diskBytes(494_000_000_000)).toBe("494 GB");
  expect(diskBytes(512)).toBe("512 B");
  expect(diskBytes(null)).toBe("—");
});

it("counts a nested suggestion once inside the one that contains it", () => {
  const items = [suggestion("/h/Library/Caches", 10), suggestion("/h/Library/Caches/Homebrew", 4), suggestion("/h/.npm", 3), suggestion("/h/Downloads", 50, "review")];
  expect(cleanupTotal(items)).toBe(63);
  expect(cleanupTotal(items, "safe")).toBe(13);
});

it("shortens home paths to a tilde", () => {
  expect(displayPath("/Users/demo/.npm", "/Users/demo")).toBe("~/.npm");
  expect(displayPath("/Users/demoX", "/Users/demo")).toBe("/Users/demoX");
});

it("round-trips a snapshot folded onto a message", () => {
  const sent = withSnapshot("  why so full?  ", "Disk: 1 GB free");
  expect(splitSnapshot(sent)).toEqual({ text: "why so full?", snapshot: "Disk: 1 GB free" });
  expect(splitSnapshot("plain")).toEqual({ text: "plain", snapshot: null });
  expect(withSnapshot("q", "")).toBe("q");
});

it("parses a storage plan and drops rows it cannot trust", () => {
  const plan = parseStoragePlan(JSON.stringify({
    items: [
      { path: "~/Library/Caches/go-build", sizeBytes: 5e9, why: "cache", safety: "safe" },
      { path: "relative/path", sizeBytes: 1 },
      { path: "/" },
      { path: "~/../etc", sizeBytes: 1 },
      { path: "/Users/demo/big.iso", sizeBytes: -4, safety: "maybe" },
    ],
    commands: [{ run: "brew cleanup", frees: 1e9 }, { run: "  " }],
  }), "/Users/demo");
  expect(plan).toEqual({
    title: "Cleanup plan",
    items: [
      { path: "/Users/demo/Library/Caches/go-build", sizeBytes: 5e9, why: "cache", safety: "safe" },
      { path: "/Users/demo/big.iso", sizeBytes: null, why: "", safety: "review" },
    ],
    commands: [{ run: "brew cleanup", why: "", frees: 1e9 }],
  });
  expect(parseStoragePlan("{\"items\": [", "/Users/demo")).toBeNull();
  expect(parseStoragePlan(JSON.stringify({ items: [] }), "/Users/demo")).toBeNull();
});
