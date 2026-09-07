import { describe, expect, it } from "vitest";
import { filterSettingsRows, STATIC_SETTINGS_ROWS, type SearchableRow } from "./settingsSearch";

const rows: SearchableRow[] = [
  { section: "permissions", label: "Auto-approve provider permissions", description: "Accept provider permission requests automatically" },
  { section: "composer", label: "Suggestion model", description: "Which model writes the inline continuation" },
  { section: "harnesses", label: "OpenCode", description: "Bridge-managed · 1.18.3" },
  { section: "harnesses", label: "Codex", description: "Your own install" },
];

describe("filterSettingsRows", () => {
  it("returns every row, grouped, when the query is empty", () => {
    const groups = filterSettingsRows("", rows);
    expect(groups.map(group => group.section)).toEqual(["permissions", "composer", "harnesses"]);
    expect(groups.flatMap(group => group.rows)).toHaveLength(4);
  });

  it("matches on the row label", () => {
    const groups = filterSettingsRows("opencode", rows);
    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map(row => row.label)).toEqual(["OpenCode"]);
  });

  it("matches on the row description, so a setting is findable by what it does", () => {
    const groups = filterSettingsRows("inline continuation", rows);
    expect(groups.flatMap(group => group.rows.map(row => row.label))).toEqual(["Suggestion model"]);
  });

  it("is case-insensitive and ignores surrounding whitespace", () => {
    expect(filterSettingsRows("  AUTO-APPROVE ", rows)[0].rows[0].label)
      .toBe("Auto-approve provider permissions");
  });

  it("names the page each hit belongs to", () => {
    const groups = filterSettingsRows("install", rows);
    expect(groups).toHaveLength(1);
    expect(groups[0].pageLabel).toBe("Harnesses");
  });

  it("returns nothing rather than everything when nothing matches", () => {
    expect(filterSettingsRows("zzzz", rows)).toEqual([]);
  });
});

describe("STATIC_SETTINGS_ROWS", () => {
  // The search is the only way some rows are reachable without browsing all
  // nine pages, so every page has to contribute at least one row to it.
  it("covers every page that has a fixed row", () => {
    const covered = new Set(STATIC_SETTINGS_ROWS.map(row => row.section));
    for (const section of ["appearance", "permissions", "composer", "agents", "models", "prompts", "harnesses", "work", "import"] as const) {
      expect(covered.has(section)).toBe(true);
    }
  });

  it("finds the inline suggestion model on Composer, not on Models", () => {
    const groups = filterSettingsRows("suggestion", STATIC_SETTINGS_ROWS);
    expect(groups.map(group => group.section)).toEqual(["composer"]);
  });
});
