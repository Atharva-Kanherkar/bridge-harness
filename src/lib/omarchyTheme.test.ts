import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { OMARCHY_TOKEN_NAMES, omarchyTokens, type OmarchyPalette } from "./omarchyTheme";

const THEMES = join(__dirname, "../../testing/fixtures/omarchy/themes");

/** Enough TOML for a flat `key = "value"` file, which is all colors.toml is. */
function loadTheme(name: string): OmarchyPalette {
  const colors: Record<string, string> = {};
  let mode: OmarchyPalette["mode"] = "dark";
  for (const line of readFileSync(join(THEMES, `${name}.toml`), "utf8").split("\n")) {
    const match = line.match(/^\s*([a-z_]+)\s*=\s*"([^"]*)"/);
    if (!match) continue;
    if (match[1] === "mode") mode = match[2] === "light" ? "light" : "dark";
    else colors[match[1]] = match[2];
  }
  return { mode, colors };
}

const themeNames = readdirSync(THEMES).filter(file => file.endsWith(".toml")).map(file => file.replace(/\.toml$/, ""));

describe("omarchyTokens", () => {
  it("covers every shipped theme with the full token set", () => {
    expect(themeNames.length).toBe(23);
    for (const name of themeNames) {
      const tokens = omarchyTokens(loadTheme(name));
      expect(Object.keys(tokens), name).toEqual([...OMARCHY_TOKEN_NAMES]);
      for (const [token, value] of Object.entries(tokens)) {
        expect(value, `${name} ${token}`).not.toMatch(/undefined/);
      }
    }
  });

  it("maps tokyo-night role for role", () => {
    const tokens = omarchyTokens(loadTheme("tokyo-night"));
    expect(tokens).toMatchObject({
      "--sidebar": "#13141c",
      "--background": "#1a1b26",
      "--card": "color-mix(in srgb, #24283b 55%, #1a1b26)",
      "--popover": "#24283b",
      "--code": "#0e0e14",
      "--foreground": "#c0caf5",
      "--body": "#a9b1d6",
      "--muted-foreground": "color-mix(in srgb, #565f89 55%, #a9b1d6)",
      "--faint": "#565f89",
      "--faint-2": "#414868",
      "--selection": "#292e42",
      "--border": "color-mix(in srgb, #a9b1d6 9%, #1a1b26)",
      "--border-card": "color-mix(in srgb, #a9b1d6 13%, #1a1b26)",
      "--primary": "#7aa2f7",
      "--primary-foreground": "#0e0e14",
      "--success": "#9ece6a",
      "--warning": "#e0af68",
      "--info": "#7aa2f7",
      "--destructive": "#f7768e",
      "--syn-keyword": "#ad8ee6",
      "--syn-number": "#eb927b",
      "--syn-operator": "#0db9d7",
      "--window-radius": "0px",
      "--radius": "0.3rem",
    });
  });

  it("never touches vendor tints", () => {
    expect(OMARCHY_TOKEN_NAMES.some(name => name.startsWith("--harness-") || name.startsWith("--chart-"))).toBe(false);
  });

  it("falls back when a theme ships without orange", () => {
    const white = loadTheme("white");
    expect(white.colors.orange).toBeUndefined();
    const tokens = omarchyTokens(white);
    expect(tokens["--syn-number"]).toBe(white.colors.bright_yellow ?? white.colors.yellow);
  });

  it("uses heavier lines on light themes", () => {
    const light = omarchyTokens(loadTheme("flexoki-light"));
    expect(light["--border"]).toMatch(/ 12%, /);
    expect(light["--input"]).toMatch(/ 16%, /);
    const dark = omarchyTokens(loadTheme("gruvbox"));
    expect(dark["--border"]).toMatch(/ 9%, /);
    expect(dark["--input"]).toMatch(/ 13%, /);
  });

  it("returns nothing without the colours everything derives from", () => {
    expect(omarchyTokens({ mode: "dark", colors: { foreground: "#ffffff", accent: "#ff0000" } })).toEqual({});
  });

  it("ignores values that are not hex colours", () => {
    const tokens = omarchyTokens({
      mode: "dark",
      colors: { background: "#000", foreground: "#fff", accent: "#123456", red: "url(x)", green: "rgb(1,2,3)" },
    });
    expect(tokens["--destructive"]).toBe("#123456");
    expect(tokens["--success"]).toBe("#123456");
  });
});
