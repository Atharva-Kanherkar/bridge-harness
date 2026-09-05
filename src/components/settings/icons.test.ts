import { readdirSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { describe, expect, it } from "vitest";

// The Settings screen is on Phosphor Regular. The rest of the app is still on
// lucide, and migrating it is a follow-up, so the boundary is what needs
// guarding: a page that reaches for a lucide symbol reintroduces two icon
// families in one column, which is exactly the drift the redesign removed.
//
// Listed by module rather than by glob, because these files are the Settings
// screen: the rail, the kit, the nine pages, and the four components the pages
// compose.

const SETTINGS_MODULES = [
  "src/components/SettingsScreen.tsx",
  "src/components/PromptStudio.tsx",
  "src/components/ManagedAgentsPanel.tsx",
  "src/components/OpenCodeHarnessSettings.tsx",
  "src/components/WorkSettingsSection.tsx",
  "src/components/ImportHarnessSection.tsx",
];

function settingsKitFiles(): string[] {
  const dir = "src/components/settings";
  return readdirSync(dir)
    .filter(name => [".ts", ".tsx"].includes(extname(name)) && !name.includes(".test."))
    .map(name => join(dir, name));
}

describe("Settings icons", () => {
  it("watches every module the Settings screen is made of", () => {
    // The list is the gate; this is the check that the list found the kit.
    const kit = settingsKitFiles();
    expect(kit.length).toBeGreaterThan(5);
    expect(kit).toContain("src/components/settings/kit.tsx");
    expect(kit).toContain("src/components/settings/SettingsRail.tsx");
  });

  it("imports no symbol from lucide-react", () => {
    const offenders = [...SETTINGS_MODULES, ...settingsKitFiles()]
      .filter(path => /from\s+["']lucide-react["']/.test(readFileSync(path, "utf8")));
    expect(offenders, "Settings uses @phosphor-icons/react, Regular weight").toEqual([]);
  });

  it("uses no native select and no native checkbox in its source", () => {
    // The DOM-level assertion lives in SettingsScreen.test.tsx; this one catches
    // a control added to a branch no test happens to render.
    const offenders = [...SETTINGS_MODULES, ...settingsKitFiles()]
      .filter(path => {
        const source = readFileSync(path, "utf8");
        return /<select\b/.test(source) || /type=["']checkbox["']/.test(source);
      });
    expect(offenders).toEqual([]);
  });
});
