/**
 * The flood: one turn, a hundred steps, in the shape the item-prefixed
 * emitter actually publishes it.
 *
 * Generated rather than written out. Four hundred hand-written frames would be
 * unreviewable, and the thing under test is a *shape* — a fresh reasoning item
 * between every tool item, four durable frames per step — not any particular
 * command. The recipe is the fixture; the numbers below are the contract.
 *
 * Frames per step mirror `codex.json` exactly:
 *
 * - a thought, as a transient delta plus a persisted completion;
 * - a command as started / output delta / completed, a file read as
 *   started / completed, a patch as started / completed.
 *
 * The turn opens with the user's message and a first thought, and closes with
 * a last thought and the reply — so a correct grouping draws five top-level
 * rows whatever the step count is.
 */

import { asWireKind } from "../wire";
import type { AgentEvent } from "../../types";

type Step = "run" | "read" | "edit";

/** What the collapsed run has to say about itself, at 100 steps. */
export const FLOOD_SUMMARY = "Ran 62 commands, read 30 files, edited 8 files";

export const FLOOD_STEPS = 100;

/**
 * Exactly 62 commands, 30 reads and 8 patches, interleaved rather than left in
 * three blocks: a run that changes verb is the case the summary line and the
 * "Explored" chunking both have to survive.
 *
 * The cycle asks for one patch in ten; the quota grants eight and spends the
 * rest on commands, which is what makes the counts exact at any step count.
 */
export function floodSteps(steps = FLOOD_STEPS): Step[] {
  const quota: Record<Step, number> = {
    run: Math.round(steps * 0.62),
    read: Math.round(steps * 0.3),
    edit: steps - Math.round(steps * 0.62) - Math.round(steps * 0.3),
  };
  const cycle: Step[] = ["run", "read", "run", "run", "read", "run", "edit", "run", "read", "run"];
  const order: Step[] = [];
  while (order.length < steps) {
    const wanted = cycle[order.length % cycle.length];
    const kind = quota[wanted] > 0 ? wanted : (["run", "read", "edit"] as Step[]).find(step => quota[step] > 0)!;
    quota[kind] -= 1;
    order.push(kind);
  }
  return order;
}

/** One turn of `steps` tool calls, as `bridgeApi.onAgentEvent` delivers it. */
export function floodStream(steps = FLOOD_STEPS): AgentEvent[] {
  const events: AgentEvent[] = [];
  let sequence = 0;

  const push = (kind: string, overrides: Partial<AgentEvent> & { persisted?: boolean } = {}) => {
    const { persisted = true, ...rest } = overrides;
    const seq = persisted ? (sequence += 1) : 0;
    events.push({
      id: persisted ? seq : 0,
      sessionId: "flood",
      sequence: seq,
      protocolVersion: 1,
      kind: asWireKind(kind),
      itemId: null, role: null, status: null, title: null, text: null,
      data: {},
      providerMeta: { adapter: "flood" },
      createdAt: new Date(Date.UTC(2026, 8, 5, 10, 0, Math.min(59, seq))).toISOString(),
      ...rest,
    });
  };

  const thought = (index: number, text: string) => {
    push("reasoning.delta", { persisted: false, itemId: `reasoning-${index}`, status: "streaming", text });
    push("reasoning.completed", {
      itemId: `reasoning-${index}`, status: "completed", text,
      data: { type: "reasoning", id: `reasoning-${index}`, text },
    });
  };

  push("message.completed", {
    itemId: "user-1", role: "user", status: "completed",
    text: "Port the runtime to the new store and keep the suite green.",
  });

  floodSteps(steps).forEach((step, index) => {
    thought(index + 1, `Step ${index + 1}: deciding what to do next.`);
    if (step === "run") {
      const command = `cargo test -p bridge-core -- case_${index + 1}`;
      push("command.started", {
        itemId: `cmd-${index}`, title: command, status: "inProgress",
        data: { type: "commandExecution", id: `cmd-${index}`, command, status: "inProgress" },
      });
      push("command.output_delta", { persisted: false, itemId: `cmd-${index}`, status: "inProgress", text: "ok\n" });
      push("command.completed", {
        itemId: `cmd-${index}`, title: command, status: "completed",
        data: { type: "commandExecution", id: `cmd-${index}`, command, status: "completed", exitCode: 0, aggregatedOutput: "ok\n", durationMs: 120 },
      });
      return;
    }
    if (step === "read") {
      const path = `src-tauri/bridge-core/src/module_${index + 1}.rs`;
      push("item.started", { itemId: `read-${index}`, status: "inProgress", data: { type: "readFile", id: `read-${index}`, path } });
      push("item.completed", {
        itemId: `read-${index}`, status: "completed", text: "pub fn run() {}\n",
        data: { type: "readFile", id: `read-${index}`, path, durationMs: 30 },
      });
      return;
    }
    const path = `src-tauri/bridge-core/src/patched_${index + 1}.rs`;
    const diff = "@@ -1,1 +1,1 @@\n-pub fn run() {}\n+pub fn run() { ok() }\n";
    push("file_change.started", {
      itemId: `patch-${index}`, status: "inProgress",
      data: { type: "fileChange", id: `patch-${index}`, changes: [{ path, diff }] },
    });
    push("file_change.completed", {
      itemId: `patch-${index}`, status: "completed",
      data: { type: "fileChange", id: `patch-${index}`, changes: [{ path, diff }], additions: 1, deletions: 1, durationMs: 60 },
    });
  });

  thought(steps + 1, "That is everything; writing it up.");
  const reply = "Ported the runtime and the suite is green.";
  push("message.delta", { persisted: false, itemId: "agent-1", role: "assistant", status: "streaming", text: reply });
  push("message.completed", {
    itemId: "agent-1", role: "assistant", status: "completed", text: reply,
    data: { type: "agentMessage", id: "agent-1", text: reply },
  });
  push("turn.completed", { persisted: false, status: "completed", data: { turn: { status: "completed" } } });

  return events;
}
