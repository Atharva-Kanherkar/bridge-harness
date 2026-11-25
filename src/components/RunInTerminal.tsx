import { createContext, useContext, useEffect, useRef, useState } from "react";
import { AlertCircle, Check, Play } from "lucide-react";
import { commandFromFence } from "../terminal/runCommand";
import type { TerminalRunResult } from "../terminal/runQueue";
import { Button } from "@/components/ui/button";

// A shell block an agent wrote, run in the chat's own terminal with one
// click. Only a host that owns a terminal provides the context: notes,
// previews and worker feeds render the block with Copy alone.
// Contract: testing/feat-run-in-terminal.md.

export type RunInTerminal = (command: string) => Promise<TerminalRunResult>;

export const RunInTerminalContext = createContext<RunInTerminal | null>(null);

const SENT_MS = 2_000;
const FAILED_MS = 3_000;

type RunState = { kind: "idle" } | { kind: "running" } | { kind: "sent"; shell: string } | { kind: "failed"; message: string };

/** The block's Run action, or nothing when this block cannot run here. */
export function RunButton({ lang, body, closed }: { lang: string; body: string; closed: boolean }) {
  const run = useContext(RunInTerminalContext);
  const [state, setState] = useState<RunState>({ kind: "idle" });
  const reset = useRef<number>();
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => { live.current = false; window.clearTimeout(reset.current); };
  }, []);

  // A fence still streaming is half a command: it never gets a Run.
  const command = run && closed ? commandFromFence(lang, body) : null;
  if (!run || command === null) return null;

  const settle = (next: RunState, ms: number) => {
    if (!live.current) return;
    setState(next);
    window.clearTimeout(reset.current);
    reset.current = window.setTimeout(() => { if (live.current) setState({ kind: "idle" }); }, ms);
  };
  const start = () => {
    if (state.kind === "running") return;
    window.clearTimeout(reset.current);
    setState({ kind: "running" });
    run(command).then(
      result => settle({ kind: "sent", shell: result.shell }, SENT_MS),
      (error: unknown) => settle({ kind: "failed", message: error instanceof Error ? error.message : String(error) }, FAILED_MS),
    );
  };

  const lines = command.split("\n").length;
  const label = state.kind === "running" ? "Run" : state.kind === "sent" ? `Sent to ${state.shell}` : state.kind === "failed" ? "Couldn't run" : "Run";
  const name = state.kind === "idle" ? "Run in terminal" : state.kind === "failed" ? `Couldn't run: ${state.message}` : label;
  const title = state.kind === "failed" ? state.message : state.kind === "idle" ? (lines > 1 ? `Run these ${lines} lines in the terminal` : "Run in the terminal") : label;

  return (
    <Button
      variant={state.kind === "failed" ? "destructive-ghost" : "ghost"}
      size="xs"
      onClick={start}
      disabled={state.kind === "running"}
      loading={state.kind === "running"}
      aria-label={name}
      tooltip={title}
      data-run-state={state.kind}
      className="group/run"
    >
      {state.kind === "idle" && (
        <span aria-hidden="true" className="grid size-4 place-items-center rounded-full bg-foreground text-background transition-transform group-hover/run:scale-110 group-active/run:scale-95">
          <Play size={8} fill="currentColor" strokeWidth={0} className="translate-x-px" />
        </span>
      )}
      {state.kind === "sent" && <Check size={12} aria-hidden="true" />}
      {state.kind === "failed" && <AlertCircle size={12} aria-hidden="true" />}
      <span aria-live="polite" className={state.kind === "running" || state.kind === "sent" ? "text-muted-foreground" : undefined}>{label}</span>
    </Button>
  );
}
