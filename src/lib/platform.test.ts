import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { formatShortcut, isMacPlatform, modKey } from "./platform";

describe("isMacPlatform", () => {
  it("is Linux or Windows only when the platform says so", () => {
    expect(isMacPlatform({ platform: "MacIntel" })).toBe(true);
    expect(isMacPlatform({ platform: "" })).toBe(true);
    expect(isMacPlatform(undefined)).toBe(true);
    expect(isMacPlatform({ platform: "Linux x86_64" })).toBe(false);
    expect(isMacPlatform({ platform: "Win32" })).toBe(false);
    expect(isMacPlatform({ platform: "", userAgentData: { platform: "Linux" } })).toBe(false);
  });
});

describe("formatShortcut", () => {
  it("prints macOS glyphs in macOS order", () => {
    expect(formatShortcut({ meta: true, shift: true }, "K", true)).toBe("⇧⌘K");
    expect(formatShortcut({ meta: true, alt: true }, "F", true)).toBe("⌥⌘F");
    expect(formatShortcut({ ctrl: true, alt: true, shift: true, meta: true }, "N", true)).toBe("⌃⌥⇧⌘N");
    expect(modKey("S", true)).toBe("⌘S");
  });

  it("prints words on Linux, with the primary modifier as Ctrl", () => {
    expect(formatShortcut({ meta: true, shift: true }, "K", false)).toBe("Ctrl+Shift+K");
    expect(formatShortcut({ meta: true, alt: true }, "1", false)).toBe("Ctrl+Alt+1");
    expect(formatShortcut({}, "F", false)).toBe("F");
    expect(modKey("P", false)).toBe("Ctrl+P");
  });
});

/** Source files under src/, minus tests and generated code. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "protocol" ? [] : sources(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

/** Drop comments: a ⌘ in prose about the macOS chord is fine; one on screen is not. */
const stripComments = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1");

describe("keycaps", () => {
  it("never hardcodes ⌘ outside the platform helper", () => {
    const root = join(__dirname, "..");
    const offenders = sources(root)
      .filter(path => !path.endsWith(join("lib", "platform.ts")))
      .filter(path => stripComments(readFileSync(path, "utf8")).includes("⌘"))
      .map(path => relative(root, path));
    expect(offenders).toEqual([]);
  });
});
