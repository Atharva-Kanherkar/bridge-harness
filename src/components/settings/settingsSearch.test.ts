import { describe, expect, it } from "vitest";
import { filterSettingsRows, PAGE_ROWS, SETTINGS_ROWS, type SearchableRow } from "./settingsSearch";
import { ALL_PAGES } from "./sections";

const rows: SearchableRow[] = [
  { page: "permissions", label: "Auto-approve provider permissions", description: "Accept provider permission requests automatically" },
  { page: "composer", label: "Suggestion model", description: "Which model writes the inline continuation" },
  { page: "codingAgents", label: "OpenCode", description: "Bridge-managed · 1.18.3", target: { kind: "harness", id: "opencode" } },
  { page: "codingAgents", label: "Codex", description: "Your own install", target: { kind: "harness", id: "codex" } },
];

describe("filterSettingsRows", () => {
  it("returns every row, grouped in rail order, when the query is empty", () => {
    const groups = filterSettingsRows("", rows);
    expect(groups.map(group => group.page)).toEqual(["composer", "codingAgents", "permissions"]);
    expect(groups.flatMap(group => group.rows)).toHaveLength(4);
  });

  it("matches on the row label and description, case-insensitively", () => {
    expect(filterSettingsRows("opencode", rows)[0].rows.map(row => row.label)).toEqual(["OpenCode"]);
    expect(filterSettingsRows("inline continuation", rows).flatMap(group => group.rows.map(row => row.label))).toEqual(["Suggestion model"]);
    expect(filterSettingsRows("  AUTO-APPROVE ", rows)[0].rows[0].label).toBe("Auto-approve provider permissions");
  });

  it("names the page each hit belongs to", () => {
    const groups = filterSettingsRows("install", rows);
    expect(groups[0].pageLabel).toBe("Coding agents");
  });

  it("returns nothing rather than everything when nothing matches", () => {
    expect(filterSettingsRows("zzzz", rows)).toEqual([]);
  });
});

describe("the search index", () => {
  it("finds every page by its name", () => {
    expect(PAGE_ROWS.map(row => row.page)).toEqual(ALL_PAGES);
  });

  it("gives every fixed row a unique anchor id", () => {
    const ids = SETTINGS_ROWS.map(row => row.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  // Contract: testing/feat-plain-browser-only.md. Browser clones are off.
  it("offers no browser clone rows", () => {
    for (const query of ["clone", "cookies", "browser copies"]) expect(filterSettingsRows(query, [...PAGE_ROWS, ...SETTINGS_ROWS])).toEqual([]);
  });

  it("finds the inline suggestion model on Typing & search, not on Models", () => {
    expect(filterSettingsRows("suggestion", SETTINGS_ROWS).map(group => group.page)).toEqual(["composer"]);
  });

  it("finds Input while running on New chats, the only place it is set", () => {
    const groups = filterSettingsRows("while running", SETTINGS_ROWS);
    expect(groups.map(group => group.page)).toEqual(["chatDefaults"]);
    expect(filterSettingsRows("active-turn", [...PAGE_ROWS, ...SETTINGS_ROWS])).toEqual([]);
  });

  it("finds the early compaction switch on Coding agents", () => {
    const groups = filterSettingsRows("compaction", [...PAGE_ROWS, ...SETTINGS_ROWS]);
    expect(groups.map(group => group.page)).toEqual(["codingAgents"]);
    expect(groups[0].rows.map(row => [row.id, row.label])).toEqual([["early-compaction", "Early compaction"]]);
  });
});

it("leaves the reviewer settings to the Reviews app", () => {
  expect(SETTINGS_ROWS.map(entry => entry.id)).not.toContain("reviewer");
  expect(SETTINGS_ROWS.map(entry => entry.id)).not.toContain("review-jobs");
});
