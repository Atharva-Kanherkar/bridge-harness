// Holding a workflow run's first turn until its connectors are up.
//
// Account connectors (`claude.ai …`) load natively and connect in the
// background: the account's list arrives, then each server connects. A turn
// sent before that sees no connector tools at all, and the run reports it
// could not reach a connector it was granted. So the run's only turn waits,
// bounded, until every granted account connector has left `pending`.
//
// While it waits it also switches off any server it was not granted that
// still came up (one that `deniedMcpServers` could not name, such as a
// connector added since Bridge last looked). The gate would refuse its tools
// anyway; switching it off keeps their schemas out of the run's context.

export const CONNECTOR_WAIT_MS = 30_000;
const POLL_MS = 250;

/**
 * Resolve once every server in `waitFor` is listed and not `pending`, or at
 * the deadline. Never rejects: a run that cannot see its connectors still
 * runs, and says so in its result.
 *
 * `run` needs `mcpServerStatus()` and `toggleMcpServer(name, enabled)`, as
 * the Agent SDK's query object has them.
 */
export async function settleWorkflowConnectors(run, {
  granted = [],
  waitFor = [],
  timeoutMs = CONNECTOR_WAIT_MS,
  pollMs = POLL_MS,
  now = Date.now,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  const allowed = new Set(granted);
  const switchedOff = new Set();
  const deadline = now() + timeoutMs;
  let statuses = [];
  while (true) {
    try {
      statuses = (await run.mcpServerStatus()) ?? [];
    } catch {
      statuses = [];
    }
    for (const server of statuses) {
      const name = server?.name;
      if (typeof name !== "string" || allowed.has(name) || switchedOff.has(name) || server.status === "disabled") continue;
      switchedOff.add(name);
      try {
        await run.toggleMcpServer(name, false);
      } catch {
        // Its tools stay refused by the gate either way.
      }
    }
    const ready = waitFor.every((name) => statuses.some((server) => server?.name === name && server.status !== "pending"));
    if (ready) return { ready: true, statuses, switchedOff: [...switchedOff] };
    if (now() >= deadline) return { ready: false, statuses, switchedOff: [...switchedOff] };
    await sleep(pollMs);
  }
}
