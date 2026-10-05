import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";

const SIDECAR = fileURLToPath(new URL("../index.mjs", import.meta.url));
const SDK = `
import { writeFileSync } from "node:fs";
export function query({ prompt, options }) {
  writeFileSync(process.env.FIXTURE_OPTIONS, JSON.stringify(options));
  return {
    async close() {
      // The installed SDK flushes its transcript before closing its process.
      await new Promise(resolve => setTimeout(resolve, 30));
      writeFileSync(process.env.FIXTURE_CLOSED, "closed");
    },
    async *[Symbol.asyncIterator]() {
      for await (const message of prompt) {
        const mode = process.env.FIXTURE_MODE;
        yield { type: "result", subtype: "success", is_error: !mode.startsWith("success"),
          session_id: options.resume ?? options.sessionId,
          result: mode === "success" ? "done" : mode === "success-auth-text" ? "Not logged in · Please run /login" : mode };
      }
    },
  };
}
`;

async function launch(t, mode, config = { sessionId: "preserved-session" }) {
  const root = await mkdtemp(join(tmpdir(), "bridge-auth-recovery-"));
  const sdk = join(root, "sdk.mjs");
  const closedPath = join(root, "closed");
  const optionsPath = join(root, "options.json");
  await writeFile(sdk, SDK);
  const child = spawn(process.execPath, [SIDECAR, JSON.stringify(config)], {
    env: { ...process.env, BRIDGE_CLAUDE_SDK_ENTRY: sdk, FIXTURE_MODE: mode,
      FIXTURE_CLOSED: closedPath, FIXTURE_OPTIONS: optionsPath },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk; });
  child.stdin.on("error", () => {});
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await exited;
    await rm(root, { recursive: true, force: true });
  });
  const send = () => child.stdin.write(JSON.stringify({ type: "user", message: { content: "hello" } }) + "\n");
  const frames = () => stdout.trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
  const waitForFrames = async count => {
    const deadline = Date.now() + 5000;
    while (frames().length < count && Date.now() < deadline) await delay(10);
    assert.equal(frames().length, count, stderr);
  };
  return { child, exited, closedPath, optionsPath, send, frames, waitForFrames, stderr: () => stderr };
}

for (const reason of [
  "Not logged in · Please run /login",
  "Failed to authenticate: OAuth session expired and could not be refreshed",
]) {
  test(`retires the stale query after ${reason}`, { timeout: 15000 }, async t => {
    const run = await launch(t, reason);
    run.send();
    const exit = await Promise.race([run.exited, delay(5000, null, { ref: false }).then(() => null)]);
    assert.deepEqual(exit, { code: 1, signal: null }, "an auth failure must retire the warm query even while stdin stays open");
    assert.equal(run.stderr(), "");
    assert.equal(await readFile(run.closedPath, "utf8"), "closed");
    assert.deepEqual(run.frames(), [{ type: "result", subtype: "success", is_error: true,
      session_id: "preserved-session", result: reason }]);
    const resumed = await launch(t, "success", { sessionId: "preserved-session", resume: true });
    resumed.send();
    await resumed.waitForFrames(1);
    assert.equal(resumed.frames()[0].session_id, "preserved-session");
    assert.equal(resumed.frames()[0].is_error, false);
    assert.equal(JSON.parse(await readFile(resumed.optionsPath, "utf8")).resume, "preserved-session");
  });
}

for (const reason of ["success", "success-auth-text", "Rate limit exceeded", "Permission denied", "Invalid API key", "Example: Not logged in · Please run /login"]) {
  test(`keeps a warm query for ${reason}`, { timeout: 15000 }, async t => {
    const run = await launch(t, reason);
    run.send();
    await run.waitForFrames(1);
    run.send();
    await run.waitForFrames(2);
    assert.equal(run.child.exitCode, null);
    run.child.stdin.end();
    assert.deepEqual(await run.exited, { code: 0, signal: null });
    assert.equal(run.stderr(), "");
  });
}
