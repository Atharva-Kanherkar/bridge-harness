import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SIDECAR = fileURLToPath(new URL("../index.mjs", import.meta.url));
const PAYLOAD_BYTES = 4 * 1024 * 1024;
const SDK_FIXTURE = `
const text = "x".repeat(${PAYLOAD_BYTES}) + "END";
export function query() {
  const mode = process.env.BRIDGE_OUTPUT_TEST_MODE;
  return {
    async supportedModels() {
      if (mode === "catalog-error") throw new Error(text);
      return [{ value: "fixture", displayName: text }];
    },
    close() {},
    async *[Symbol.asyncIterator]() {
      if (mode === "error") throw new Error(text);
      yield { type: "assistant", message: { content: [{ type: "text", text }] } };
      yield { type: "result", subtype: "success", result: "complete" };
    },
  };
}
`;

async function runSidecar(t, mode, { config, closeOutput = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), "bridge-sidecar-output-"));
  const sdk = join(root, "sdk.mjs");
  await writeFile(sdk, SDK_FIXTURE);
  const child = spawn(process.execPath, [SIDECAR, config ?? JSON.stringify({ catalog: mode.startsWith("catalog") })], {
    env: { ...process.env, BRIDGE_CLAUDE_SDK_ENTRY: sdk, BRIDGE_OUTPUT_TEST_MODE: mode },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await closed.catch(() => {});
    await rm(root, { recursive: true, force: true });
  });
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
  const chunks = [];
  if (closeOutput) {
    child.stdout.destroy();
  } else {
    // Keep stdin open and let the stdout pipe fill before reading. Each chunk
    // then yields, like a parent busy persisting the preceding provider event.
    // Attach immediately: Node drains unconsumed child streams on early exit.
    child.stdout.on("data", (chunk) => {
      chunks.push(chunk);
      child.stdout.pause();
      setTimeout(() => child.stdout.resume(), 1);
    });
    child.stdout.pause();
    await delay(50);
    child.stdout.resume();
  }
  const result = await closed;
  return { ...result, stderr, stdout: Buffer.concat(chunks).toString("utf8") };
}

function frames(result, expectedCode) {
  assert.equal(result.signal, null);
  assert.equal(result.code, expectedCode);
  assert.equal(result.stderr, "");
  assert.ok(result.stdout.endsWith("\n"), "the final NDJSON frame must be complete");
  return result.stdout.trimEnd().split("\n").map((line) => JSON.parse(line));
}

test("a slow parent receives the complete large message and final result in order", { timeout: 15000 }, async (t) => {
  const output = frames(await runSidecar(t, "success"), 0);
  assert.equal(output.length, 2);
  assert.equal(output[0].type, "assistant");
  assert.equal(output[0].message.content[0].text.length, PAYLOAD_BYTES + 3);
  assert.ok(output[0].message.content[0].text.endsWith("END"));
  assert.deepEqual(output[1], { type: "result", subtype: "success", result: "complete" });
});

test("a large catalogue is fully delivered before successful exit", { timeout: 15000 }, async (t) => {
  const output = frames(await runSidecar(t, "catalog"), 0);
  assert.equal(output.length, 1);
  assert.equal(output[0].type, "model_catalog");
  assert.equal(output[0].models[0].displayName.length, PAYLOAD_BYTES + 3);
  assert.ok(output[0].models[0].displayName.endsWith("END"));
});

for (const mode of ["error", "catalog-error"]) {
  test(`a large ${mode} is fully delivered before failure exit`, { timeout: 15000 }, async (t) => {
    const output = frames(await runSidecar(t, mode), 1);
    assert.equal(output.length, 1);
    assert.equal(output[0].type, "result");
    assert.equal(output[0].subtype, "error_sidecar");
    assert.equal(output[0].is_error, true);
    assert.ok(output[0].result.length > PAYLOAD_BYTES);
    assert.ok(output[0].result.endsWith("END"));
  });
}

test("invalid configuration still returns a typed error before exiting", { timeout: 15000 }, async (t) => {
  const [output] = frames(await runSidecar(t, "success", { config: "{" }), 1);
  assert.equal(output.subtype, "error_sidecar");
  assert.match(output.result, /^Invalid sidecar config:/);
});

test("a closed parent output pipe terminates the sidecar", { timeout: 15000 }, async (t) => {
  const result = await runSidecar(t, "success", { closeOutput: true });
  assert.equal(result.signal, null);
  assert.equal(result.code, 1);
});
