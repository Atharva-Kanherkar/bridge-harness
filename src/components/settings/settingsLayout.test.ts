import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SETTINGS_COLUMN, SETTINGS_COLUMN_WIDE } from "./layout";

// Settings pages draw text through the app's type scale (text-ui, text-meta,
// text-caption) and the kit's controls. Raw text-xs and text-sm slipped in one
// page at a time and left the screen in three body sizes.

const DIR = __dirname;
const PAGE_FILES = [
  ...readdirSync(DIR).filter(name => name.endsWith(".tsx") && !name.endsWith(".test.tsx") && name !== "kit.tsx").map(name => join(DIR, name)),
  join(DIR, "../../features/menu-bar/MenuBarSettingsPage.tsx"),
  join(DIR, "../../features/menu-bar/MenuBarLayoutEditor.tsx"),
];

describe("settings layout", () => {
  it("centers the content column so a wide window has even margins", () => {
    for (const column of [SETTINGS_COLUMN, SETTINGS_COLUMN_WIDE]) {
      expect(column).toContain("mx-auto");
      expect(column).not.toMatch(/\bmd:pl-/);
    }
  });

  it("keeps settings pages on the type scale", () => {
    const offenders = PAGE_FILES.flatMap(file => {
      const text = readFileSync(file, "utf8");
      return [...text.matchAll(/(?<![\w-])text-(?:xs|sm)(?![\w-])/g)].map(() => file.replace(`${DIR}/`, ""));
    });
    expect(offenders).toEqual([]);
  });
});
