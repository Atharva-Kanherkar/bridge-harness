import { expect, it } from "vitest";
import { cleanupTotal, diskBytes, displayPath } from "./diskSpace";
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
