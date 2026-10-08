import { describe, expect, it } from "vitest";
import specs from "../../testing/fixtures/visualize/specs.json";
import forms from "../../testing/fixtures/visualize/forms.json";
import { FORMS } from "./catalog";
import { parseVisualSpec, validateVisualSpec } from "./spec";

interface Fixture {
  name: string;
  spec: unknown;
  valid: boolean;
  errorPaths?: string[];
}

const fixtures = specs as Fixture[];

describe("VisualSpec validation", () => {
  it("the catalog matches the shared forms fixture", () => {
    expect(FORMS).toEqual(forms);
  });

  it.each(fixtures.map(fixture => [fixture.name, fixture] as const))("agrees with the Rust validator: %s", (_, fixture) => {
    const errors = validateVisualSpec(fixture.spec);
    expect(errors.length === 0, JSON.stringify(errors)).toBe(fixture.valid);
    if (fixture.errorPaths) {
      expect(errors.map(error => error.path).sort()).toEqual([...fixture.errorPaths].sort());
    }
  });

  it("every available form has a valid golden", () => {
    for (const info of FORMS.filter(entry => entry.available)) {
      const covered = fixtures.some(fixture => fixture.valid && (fixture.spec as { blocks: { family: string; form: string }[] }).blocks.some(block => block.family === info.family && block.form === info.form));
      expect(covered, `${info.family}/${info.form}`).toBe(true);
    }
  });

  it("narrows a valid spec and refuses an invalid one", () => {
    const valid = fixtures.find(fixture => fixture.valid)!;
    const parsed = parseVisualSpec(valid.spec);
    expect(parsed.ok).toBe(true);
    const refused = parseVisualSpec({ version: 1 });
    expect(refused.ok).toBe(false);
  });

  it("counts characters the way Rust does", () => {
    const title = "é".repeat(80);
    expect(validateVisualSpec({ ...(fixtures[0].spec as object), title })).toEqual([]);
    expect(validateVisualSpec({ ...(fixtures[0].spec as object), title: `${title}é` }).map(error => error.path)).toEqual(["title"]);
  });
});
