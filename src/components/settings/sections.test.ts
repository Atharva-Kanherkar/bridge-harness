import { describe, expect, it } from "vitest";
import { ALL_SECTIONS, PRIMARY_SECTIONS, SECTION_ORDER, primarySection } from "./sections";

describe("settings destinations", () => {
  it("exposes five destinations in the rail, Import last", () => {
    expect(SECTION_ORDER.flatMap(group => group.sections)).toEqual(["general", "codingAgents", "permissions", "data", "import"]);
  });
  it("keeps every contextual link associated with a visible destination", () => {
    for (const section of ALL_SECTIONS) expect(PRIMARY_SECTIONS).toContain(primarySection(section));
    expect(primarySection("menuBar")).toBe("general");
    expect(primarySection("notifications")).toBe("general");
    expect(primarySection("prompts")).toBe("codingAgents");
    // Browser clones are off, so they are not a settings destination.
    expect(ALL_SECTIONS as string[]).not.toContain("clones");
    expect(primarySection("keys")).toBe("permissions");
    expect(primarySection("import")).toBe("import");
    expect(primarySection("archives")).toBe("data");
  });
});
