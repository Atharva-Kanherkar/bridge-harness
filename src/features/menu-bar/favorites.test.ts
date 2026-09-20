import { expect, it } from "vitest";
import { chooseFavorite, defaultFavorites } from "./favorites";

it("swaps an existing favorite without duplicates", () => {
  expect(chooseFavorite(defaultFavorites, 0, "cursor")).toEqual(["cursor", "claude"]);
  expect(defaultFavorites).toEqual(["codex", "claude"]);
  expect(chooseFavorite(defaultFavorites, 1, "opencode")).toEqual(["codex", "opencode"]);
});

it("removes and adds favorites without leaving empty slots", () => {
  expect(chooseFavorite(defaultFavorites, 1, "")).toEqual(["codex"]);
  expect(chooseFavorite(["codex"], 1, "opencode")).toEqual(["codex", "opencode"]);
  expect(chooseFavorite(["codex"], 1, "codex")).toEqual(["codex"]);
  expect(chooseFavorite(defaultFavorites, 4, "opencode")).toEqual(defaultFavorites);
});

it("adds a third favorite and removes it without changing the defaults", () => {
  const three = chooseFavorite(defaultFavorites, 2, "opencode");
  expect(three).toEqual(["codex", "claude", "opencode"]);
  expect(chooseFavorite(three, 2, "")).toEqual(defaultFavorites);
});
