import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { bridgeApi } from "../api";

const scrollback = new Map<string, string>();

export function TerminalPane({ workspaceId }: { workspaceId?: string }) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!host.current || !workspaceId) return;
    const term = new Terminal({
      fontFamily: "'SFMono-Regular', 'SF Mono', Menlo, monospace", fontSize: 13, lineHeight: 1.42,
      cursorBlink: true, cursorStyle: "bar", convertEol: true,
      theme: {
        background: "#0c0c0d",
        foreground: "#b8b8bc",
        cursor: "#e8e8ec",
        selectionBackground: "rgba(255,255,255,0.12)",
        black: "#0c0c0d",
        brightBlack: "#5c5c63",
        green: "#8a8a90",
        brightGreen: "#b0b0b6",
        yellow: "#d9b861",
        blue: "#78a9d1",
        cyan: "#74b8ad",
        white: "#e8e8ec",
        brightWhite: "#f4f4f6",
      }
    });
    const fit = new FitAddon(); term.loadAddon(fit); term.open(host.current); fit.fit();
    const previous = scrollback.get(workspaceId); if (previous) term.write(previous);
    else if (!("__TAURI_INTERNALS__" in window)) term.writeln("\x1b[90mBridge workspace shell · terminal is isolated from the agent conversation.\x1b[0m\r\n$ ");
    void bridgeApi.openTerminal(workspaceId).catch(error => term.writeln(`\r\n\x1b[31m${String(error)}\x1b[0m`));
    const data = term.onData(value => void bridgeApi.writeTerminal(workspaceId, value));
    const resize = new ResizeObserver(() => { fit.fit(); void bridgeApi.resizeTerminal(workspaceId, term.rows, term.cols); }); resize.observe(host.current);
    let unlisten: (() => void) | undefined;
    void bridgeApi.onTerminal(chunk => {
      const buffered = `${scrollback.get(chunk.sessionId) ?? ""}${chunk.data}`; scrollback.set(chunk.sessionId, buffered.slice(-1_000_000));
      if (chunk.sessionId === workspaceId) term.write(chunk.data);
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); resize.disconnect(); data.dispose(); term.dispose(); };
  }, [workspaceId]);

  if (!workspaceId) return <div className="absolute inset-0 grid place-items-center text-muted-foreground/85 text-xs">Select a workspace to open its shell.</div>;
  return <div className="absolute inset-0 p-[14px_12px] [&_.xterm]:h-full [&_.xterm-viewport]:scrollbar-thin [&_.xterm-viewport]:scrollbar-thumb-foreground/12 [&_.xterm-viewport]:scrollbar-track-transparent" ref={host} aria-label="Workspace terminal" />;
}
