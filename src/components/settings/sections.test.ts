import { describe, expect, it } from "vitest";
import { ALL_SECTIONS, PRIMARY_SECTIONS, SECTION_ORDER, primarySection } from "./sections";

describe("settings destinations", () => {
  it("exposes only four destinations in the rail", () => {
    expect(SECTION_ORDER.flatMap(group => group.sections)).toEqual(["general", "codingAgents", "permissions", "data"]);
  });
  it("keeps every contextual link associated with a visible destination", () => {
    for (const section of ALL_SECTIONS) expect(PRIMARY_SECTIONS).toContain(primarySection(section));
    expect(primarySection("menuBar")).toBe("general");
    expect(primarySection("prompts")).toBe("codingAgents");
    expect(primarySection("clones")).toBe("permissions");
    expect(primarySection("import")).toBe("data");
  });
});
