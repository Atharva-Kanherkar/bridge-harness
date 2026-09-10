import type { ITheme } from "@xterm/xterm";

/** First design token that resolves on the live document, or undefined. */
function token(styles: CSSStyleDeclaration, ...names: string[]): string | undefined {
  for (const name of names) {
    const value = styles.getPropertyValue(name).trim();
    if (value) return value;
  }
  return undefined;
}

/** Selection needs alpha, and xterm only parses hex/rgb — so derive it here. */
function translucent(color: string | undefined, alpha: number): string | undefined {
  if (!color) return undefined;
  const hex = color.replace("#", "");
  const full = hex.length === 3 ? [...hex].map(digit => digit + digit).join("") : hex;
  if (!/^[\da-f]{6}$/i.test(full)) return color;
  const value = Number.parseInt(full, 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}

// xterm paints to a canvas and cannot read CSS variables, so the palette is
// resolved from the tokens on <html> and rebuilt whenever the theme flips.
export function terminalTheme(): ITheme {
  const styles = getComputedStyle(document.documentElement);
  const surface = token(styles, "--code", "--background");
  const foreground = token(styles, "--foreground");
  return {
    background: surface,
    foreground,
    cursor: foreground,
    cursorAccent: surface,
    selectionBackground: translucent(foreground, 0.24),
    black: token(styles, "--muted"),
    brightBlack: token(styles, "--syn-comment"),
    red: token(styles, "--destructive"),
    brightRed: token(styles, "--destructive"),
    green: token(styles, "--success"),
    brightGreen: token(styles, "--syn-string"),
    yellow: token(styles, "--warning"),
    brightYellow: token(styles, "--syn-number"),
    blue: token(styles, "--info"),
    brightBlue: token(styles, "--syn-function"),
    magenta: token(styles, "--syn-keyword"),
    brightMagenta: token(styles, "--syn-type"),
    cyan: token(styles, "--syn-tag"),
    brightCyan: token(styles, "--syn-tag"),
    white: token(styles, "--muted-foreground"),
    brightWhite: foreground,
  };
}


export function terminalSearchDecorations() {
  const theme = terminalTheme();
  return {
    matchBackground: theme.selectionBackground ?? "transparent",
    matchOverviewRuler: theme.yellow ?? "transparent",
    activeMatchBackground: theme.foreground ?? "transparent",
    activeMatchForeground: theme.background ?? "transparent",
    activeMatchColorOverviewRuler: theme.foreground ?? "transparent",
  };
}
