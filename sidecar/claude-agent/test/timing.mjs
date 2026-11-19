import { test } from "node:test";
import assert from "node:assert/strict";
import { createTurnTiming } from "../timing.mjs";

test("marks each boundary once, from the user frame", () => {
  let clock = 1000;
  const timing = createTurnTiming({ sdkImportMs: 40 }, () => clock);
  timing.observe({ type: "system", subtype: "init" });
  assert.equal(timing.finish(), null, "nothing to report before a user frame");

  timing.userFrame();
  clock = 1050; timing.observe({ type: "system", subtype: "init" });
  clock = 1300; timing.observe({ type: "system", subtype: "status", status: "requesting" });
  clock = 2100; timing.observe({ type: "stream_event", event: { type: "message_start" } });
  clock = 2400; timing.observe({ type: "stream_event", event: { type: "content_block_start", content_block: { type: "thinking" } } });
  clock = 2410; timing.observe({ type: "stream_event", event: { type: "content_block_delta" } });
  clock = 2600; timing.observe({ type: "system", subtype: "status", status: "requesting" });
  clock = 3000;
  const frame = timing.finish();
  assert.deepEqual(frame, {
    type: "system",
    subtype: "bridge_timing",
    turn: { initMs: 50, requestingMs: 300, firstByteMs: 1100, firstBlockMs: 1400, firstDeltaMs: 1410, resultMs: 2000, requests: 2 },
    boot: { sdkImportMs: 40 },
  });
});

test("boot is reported with the first turn only, and steering does not restart a turn", () => {
  let clock = 0;
  const timing = createTurnTiming({ sdkImportMs: 1 }, () => clock);
  timing.userFrame();
  clock = 10; timing.userFrame();
  clock = 20;
  assert.equal(timing.finish().turn.resultMs, 20);
  timing.userFrame();
  assert.equal("boot" in timing.finish(), false);
});

import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const SIDECAR = fileURLToPath(new URL("../index.mjs", import.meta.url));
const SDK = `
export function query({ prompt }) {
  return {
    async *[Symbol.asyncIterator]() {
      for await (const message of prompt) {
        yield { type: "system", subtype: "status", status: "requesting" };
        yield { type: "stream_event", event: { type: "message_start" } };
        yield { type: "result", subtype: "success", is_error: false, result: "ok" };
      }
    },
  };
}
`;

async function oneTurn(t, config) {
  const root = await mkdtemp(join(tmpdir(), "bridge-timing-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sdk = join(root, "sdk.mjs");
  await writeFile(sdk, SDK);
  const child = spawn(process.execPath, [SIDECAR, JSON.stringify(config)], {
    env: { ...process.env, BRIDGE_CLAUDE_SDK_ENTRY: sdk },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
  const exited = new Promise(resolve => child.once("close", resolve));
  child.stdin.write(JSON.stringify({ type: "user", message: { content: "hi" } }) + "\n");
  child.stdin.end();
  await exited;
  return stdout.trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
}

test("a chat launch reports each turn after its result", async t => {
  const frames = await oneTurn(t, { sessionId: "s", timing: true });
  assert.deepEqual(
    frames.map(frame => (frame.type === "system" ? frame.subtype : frame.type)),
    ["status", "stream_event", "result", "bridge_timing"],
  );
  const timing = frames.at(-1);
  assert.equal(timing.turn.requests, 1);
  assert.equal(typeof timing.turn.firstByteMs, "number");
  assert.equal(typeof timing.boot.sdkImportMs, "number");
});

test("a background run keeps its exact frames", async t => {
  const frames = await oneTurn(t, { sessionId: "s" });
  assert.equal(frames.some(frame => frame.subtype === "bridge_timing"), false);
});
