import { describe, expect, it } from "vitest";
import { ALL_PAGES, canonicalSection, PAGES, RAIL_GROUPS, sectionLabel } from "./sections";

describe("settings pages", () => {
  it("puts every page in the rail, grouped, with Updates in the footer", () => {
    expect(RAIL_GROUPS.map(group => group.label)).toEqual(["Basics", "Agents & models", "Data", "Advanced"]);
    expect(RAIL_GROUPS.map(group => group.pages)).toEqual([
      ["general", "notifications", "voice", "composer", "menuBar"],
      ["chatDefaults", "codingAgents", "models", "agents", "permissions", "keys"],
      ["import", "archives", "data"],
      ["workers", "prompts", "work"],
    ]);
    expect(ALL_PAGES).toEqual([...RAIL_GROUPS.flatMap(group => group.pages), "updates"]);
    expect(new Set(ALL_PAGES).size).toBe(Object.keys(PAGES).length);
  });

  it("keeps retired ids as aliases so old deep links still open the right page", () => {
    expect(canonicalSection("appearance")).toBe("general");
    expect(canonicalSection("storage")).toBe("data");
    expect(canonicalSection("harnesses")).toBe("codingAgents");
    expect(canonicalSection("history")).toBe("import");
    expect(canonicalSection("keys")).toBe("keys");
    // Browser clones are off, so they are not a settings page.
    expect(ALL_PAGES as string[]).not.toContain("clones");
  });

  it("names pages the way the rail does", () => {
    expect(sectionLabel("composer")).toBe("Typing & search");
    expect(sectionLabel("menuBar")).toBe("Menu bar");
    expect(sectionLabel("work")).toBe("Daily briefing");
    expect(sectionLabel("storage")).toBe("Storage");
    expect(PAGES.general.description).not.toBe(PAGES.composer.description);
  });
});
