// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { terminalTheme } from "./theme";

// Contract: testing/fix-terminal-sweep.md §5, with the light and dark tokens
// src/index.css defines.
const LIGHT = { "--code": "#f3f3f1", "--background": "#fbfbfa", "--foreground": "#18181a", "--muted": "#f1f1ef", "--muted-foreground": "#66666b", "--warning": "#6f6862", "--syn-number": "#a15c05", "--syn-param": "#7a5320", "--syn-comment": "#6b6b60" };
const DARK = { "--code": "#0f0f11", "--background": "#0c0c0d", "--foreground": "#ededef", "--muted": "#1c1c1f", "--muted-foreground": "#8e8e94", "--warning": "#a8a29b", "--syn-number": "#f0bd7a", "--syn-param": "#e3c49c", "--syn-comment": "#7c7c72" };

function tokens(values: Record<string, string>) {
  const root = document.documentElement;
  root.removeAttribute("style");
  for (const [name, value] of Object.entries(values)) root.style.setProperty(name, value);
}

afterEach(() => { document.documentElement.removeAttribute("style"); });

describe("terminalTheme", () => {
  it("keeps ANSI black dark on a light surface", () => {
    tokens(LIGHT);
    const theme = terminalTheme();
    expect(theme.background).toBe(LIGHT["--code"]);
    expect(theme.black).toBe(LIGHT["--foreground"]);
    expect(theme.black).not.toBe(LIGHT["--muted"]);
  });

  it("keeps the conventional dark black on a dark surface", () => {
    tokens(DARK);
    expect(terminalTheme().black).toBe(DARK["--muted"]);
  });

  it("draws yellow as a colour, distinct from white, in both modes", () => {
    for (const values of [LIGHT, DARK]) {
      tokens(values);
      const theme = terminalTheme();
      expect(theme.yellow).toBe(values["--syn-number"]);
      expect(theme.yellow).not.toBe(theme.white);
      expect(theme.yellow).not.toBe(values["--warning"]);
    }
  });
});
