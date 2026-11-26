// Runs the chat asks the dock terminal to make. The pane is lazy and may not
// be mounted when a code block's Run is clicked, so a request waits here
// until a pane for its workspace picks it up. A request nobody picks up in
// time fails instead of running minutes later, when the person has moved on.
// Contract: testing/feat-run-in-terminal.md.

import { errorMessage } from "../errors";

export const TERMINAL_RUN_TIMEOUT_MS = 10_000;

export type TerminalRunResult = { shell: string };

export type TerminalRun = {
  readonly workspaceId: string;
  readonly command: string;
  /** False once the run settled or timed out: a pane must not write after. */
  readonly open: () => boolean;
  readonly settle: (shell: string) => void;
  readonly fail: (error: unknown) => void;
};

type Listener = (run: TerminalRun) => void;

const pending: TerminalRun[] = [];
const listeners = new Map<string, Listener>();

/** Ask the workspace's terminal to run `command`. Resolves with the shell it
 *  ran in, rejects when it could not run or no pane took it in time. */
export function requestTerminalRun(workspaceId: string, command: string, timeoutMs = TERMINAL_RUN_TIMEOUT_MS): Promise<TerminalRunResult> {
  return new Promise((resolve, reject) => {
    let done = false;
    const finish = () => {
      done = true;
      window.clearTimeout(timer);
      const index = pending.indexOf(run);
      if (index >= 0) pending.splice(index, 1);
    };
    const run: TerminalRun = {
      workspaceId,
      command,
      open: () => !done,
      settle: shell => { if (done) return; finish(); resolve({ shell }); },
      fail: error => { if (done) return; finish(); reject(error instanceof Error ? error : new Error(errorMessage(error))); }, // raw-error: rethrown to the caller, not shown
    };
    const timer = window.setTimeout(() => run.fail(new Error("The terminal did not open in time.")), timeoutMs);
    const listener = listeners.get(workspaceId);
    if (listener) listener(run);
    else pending.push(run);
  });
}

/** A pane takes its workspace's runs: the queued ones now, the rest as they
 *  come. The most recent pane for a workspace wins. */
export function onTerminalRun(workspaceId: string, listener: Listener): () => void {
  listeners.set(workspaceId, listener);
  for (const run of pending.filter(item => item.workspaceId === workspaceId)) {
    pending.splice(pending.indexOf(run), 1);
    listener(run);
  }
  return () => { if (listeners.get(workspaceId) === listener) listeners.delete(workspaceId); };
}
