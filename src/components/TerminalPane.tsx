import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { bridgeApi } from "../api";
import type { Session } from "../types";

export function TerminalPane({ session }: { session?: Session }) {
  const host = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal>();

  useEffect(() => {
    if (!host.current) return;
    const term = new Terminal({
      fontFamily: "'SFMono-Regular', 'SF Mono', Menlo, monospace", fontSize: 13, lineHeight: 1.42,
      cursorBlink: true, cursorStyle: "bar", convertEol: true,
      theme: { background: "#0b0d0f", foreground: "#c7cbc7", cursor: "#d8ff63", selectionBackground: "#384122", black: "#111315", brightBlack: "#606660", green: "#a8c751", brightGreen: "#d8ff63", yellow: "#d9b861", blue: "#78a9d1", cyan: "#74b8ad", white: "#c7cbc7", brightWhite: "#f0f2ee" }
    });
    const fit = new FitAddon(); term.loadAddon(fit); term.open(host.current); fit.fit(); terminal.current = term;
    if (!session) {
      term.writeln("\x1b[90m  Select a workspace to open its session.\x1b[0m");
    } else if (!("__TAURI_INTERNALS__" in window)) {
      term.writeln(`\x1b[90m╭─ \x1b[32m${session.label}\x1b[90m · supervised session\x1b[0m`);
      term.writeln("\x1b[90m│\x1b[0m I’m implementing the session supervisor and event ledger now.");
      term.writeln("\x1b[90m│\x1b[0m");
      term.writeln("\x1b[90m│\x1b[0m \x1b[32m✓\x1b[0m Added typed workspace lifecycle");
      term.writeln("\x1b[90m│\x1b[0m \x1b[32m✓\x1b[0m Wired PTY output to the Deck");
      term.writeln("\x1b[90m│\x1b[0m \x1b[33m◆\x1b[0m Running integration tests…");
      term.writeln("\x1b[90m╰─\x1b[0m");
    }
    const data = term.onData(value => { if (session) void bridgeApi.writeSession(session.id, value); });
    const resize = new ResizeObserver(() => { fit.fit(); if (session) void bridgeApi.resizeSession(session.id, term.rows, term.cols); });
    resize.observe(host.current);
    let unlisten: (() => void) | undefined;
    void bridgeApi.onTerminal(chunk => { if (chunk.sessionId === session?.id) term.write(chunk.data); }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); resize.disconnect(); data.dispose(); term.dispose(); terminal.current = undefined; };
  }, [session?.id]);

  return <div className="terminal-host" ref={host} aria-label="Live agent terminal" />;
}
