import { expect, it } from "vitest";
import { chooseFavorite, defaultFavorites } from "./favorites";

it("swaps an existing favorite without duplicates and preserves the three slots", () => {
  expect(chooseFavorite(defaultFavorites, 0, "cursor")).toEqual(["cursor", "claude", "codex"]);
  expect(defaultFavorites).toEqual(["codex", "claude", "cursor"]);
  expect(chooseFavorite(defaultFavorites, 2, "opencode")).toEqual(["codex", "claude", "opencode"]);
});

it("removes and adds favorites without leaving empty slots", () => {
  expect(chooseFavorite(defaultFavorites, 1, "")).toEqual(["codex", "cursor"]);
  expect(chooseFavorite(["codex"], 2, "opencode")).toEqual(["codex", "opencode"]);
  expect(chooseFavorite(["codex"], 1, "codex")).toEqual(["codex"]);
  expect(chooseFavorite(defaultFavorites, 4, "opencode")).toEqual(defaultFavorites);
});

it("adds a fourth favorite and removes it without changing the first three", () => {
  const four = chooseFavorite(defaultFavorites, 3, "opencode");
  expect(four).toEqual(["codex", "claude", "cursor", "opencode"]);
  expect(chooseFavorite(four, 3, "")).toEqual(defaultFavorites);
});
