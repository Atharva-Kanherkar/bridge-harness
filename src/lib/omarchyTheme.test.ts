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

  it("falls back the way Omarchy does when a theme ships without orange", () => {
    const white = loadTheme("white");
    expect(white.colors.orange).toBeUndefined();
    // omarchy-theme-color: orange = yellow.
    expect(omarchyTokens(white)["--syn-number"]).toBe(white.colors.yellow);
  });

  it("reads legacy palettes the way Omarchy resolves them", () => {
    // The exact shape omarchy-theme-colors-from-alacritty writes.
    const ansi = ["#1d2021", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
      "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"];
    const colors: Record<string, string> = { background: "#282828", foreground: "#d4be98", accent: "#7daea3", selection: "#3c3836" };
    ansi.forEach((hex, i) => { colors[`color${i}`] = hex; });
    const tokens = omarchyTokens({ mode: "dark", colors });
    expect(tokens["--destructive"]).toBe("#cc241d");
    expect(tokens["--success"]).toBe("#98971a");
    expect(tokens["--warning"]).toBe("#d79921");
    expect(tokens["--info"]).toBe("#458588");
    expect(tokens["--syn-keyword"]).toBe("#b16286");
    expect(tokens["--syn-tag"]).toBe("#689d6a");
    expect(tokens["--syn-operator"]).toBe("#8ec07c");
    expect(tokens["--syn-number"]).toBe("#d79921");
    expect(tokens["--foreground"]).toBe("#ebdbb2");
    expect(tokens["--faint"]).toBe("#928374");
    expect(tokens["--popover"]).toBe("#1d2021");
    // Shades Omarchy derives: background mixed 25% and 50% toward black.
    expect(tokens["--sidebar"]).toBe("#1e1e1e");
    expect(tokens["--code"]).toBe("#141414");
  });

  it("reads short legacy names and ANSI-only backgrounds", () => {
    const short = omarchyTokens({ mode: "dark", colors: { bg: "#101010", fg: "#e0e0e0", accent: "#ff8800", purple: "#aa55ff" } });
    expect(short["--background"]).toBe("#101010");
    expect(short["--body"]).toBe("#e0e0e0");
    expect(short["--syn-keyword"]).toBe("#aa55ff");
    const ansiOnly = omarchyTokens({ mode: "dark", colors: { color0: "#000000", color7: "#cccccc", color4: "#3366ff" } });
    expect(ansiOnly["--background"]).toBe("#000000");
    expect(ansiOnly["--primary"]).toBe("#3366ff");
  });

  it("leaves every shipped theme's own keys untouched", () => {
    for (const name of themeNames) {
      const theme = loadTheme(name);
      const tokens = omarchyTokens(theme);
      expect(tokens["--background"], name).toBe(theme.colors.background);
      expect(tokens["--sidebar"], name).toBe(theme.colors.dark_background);
      expect(tokens["--destructive"], name).toBe(theme.colors.red);
    }
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
