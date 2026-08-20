import { useEffect, useRef } from "react";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { bridgeApi } from "../api";
import { rememberScrollback, scrollbackFor } from "../terminalScrollback";

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
// Status slots take the meaning colors, the rest take the syntax scale, and
// black→brightWhite is the achromatic ramp — so nothing goes invisible when
// the theme inverts.
function terminalTheme(): ITheme {
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

export function TerminalPane({ workspaceId }: { workspaceId?: string }) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!host.current || !workspaceId) return;
    const term = new Terminal({
      fontFamily: "'SFMono-Regular', 'SF Mono', Menlo, monospace", fontSize: 13, lineHeight: 1.42,
      cursorBlink: true, cursorStyle: "bar", convertEol: true,
      theme: terminalTheme(),
    });
    const fit = new FitAddon(); term.loadAddon(fit); term.open(host.current); fit.fit();
    const theme = new MutationObserver(() => { term.options.theme = terminalTheme(); });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "data-theme"] });
    const previous = scrollbackFor(workspaceId); if (previous) term.write(previous);
    else if (!("__TAURI_INTERNALS__" in window)) term.writeln("\x1b[90mBridge workspace shell · terminal is isolated from the agent conversation.\x1b[0m\r\n$ ");
    void bridgeApi.openTerminal(workspaceId).catch(error => term.writeln(`\r\n\x1b[31m${String(error)}\x1b[0m`));
    const data = term.onData(value => void bridgeApi.writeTerminal(workspaceId, value));
    const resize = new ResizeObserver(() => { fit.fit(); void bridgeApi.resizeTerminal(workspaceId, term.rows, term.cols); }); resize.observe(host.current);
    let unlisten: (() => void) | undefined;
    void bridgeApi.onTerminal(chunk => {
      rememberScrollback(chunk.sessionId, chunk.data);
      if (chunk.sessionId === workspaceId) term.write(chunk.data);
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); theme.disconnect(); resize.disconnect(); data.dispose(); term.dispose(); };
  }, [workspaceId]);

  if (!workspaceId) return <div className="absolute inset-0 grid place-items-center text-muted-foreground/85 text-xs">Select a workspace to open its shell.</div>;
  return <div className="absolute inset-0 p-[14px_12px] [&_.xterm]:h-full [&_.xterm-viewport]:scrollbar-thin [&_.xterm-viewport]:scrollbar-thumb-foreground/12 [&_.xterm-viewport]:scrollbar-track-transparent" ref={host} aria-label="Workspace terminal" />;
}
