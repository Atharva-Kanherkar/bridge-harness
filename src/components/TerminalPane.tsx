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
      theme: { background: "#0b0d0f", foreground: "#c7cbc7", cursor: "#d8ff63", selectionBackground: "#384122", black: "#111315", brightBlack: "#606660", green: "#a8c751", brightGreen: "#d8ff63", yellow: "#d9b861", blue: "#78a9d1", cyan: "#74b8ad", white: "#c7cbc7", brightWhite: "#f0f2ee" }
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

  if (!workspaceId) return <div className="terminal-empty">Select a workspace to open its shell.</div>;
  return <div className="terminal-host" ref={host} aria-label="Workspace terminal" />;
}
