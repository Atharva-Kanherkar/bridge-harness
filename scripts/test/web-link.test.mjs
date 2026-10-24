import { test } from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs, readToken, webLink } from "../web-link.mjs";

const script = fileURLToPath(new URL("../web-link.mjs", import.meta.url));
const TOKEN = "ab".repeat(32);

function dataDir(content) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-web-link-"));
  if (content !== undefined) {
    writeFileSync(join(dir, "daemon.token"), content);
    chmodSync(join(dir, "daemon.token"), 0o600);
  }
  return dir;
}

test("builds the link with the daemon and token in the fragment", () => {
  const link = webLink({ app: "http://127.0.0.1:1420", daemon: "ws://127.0.0.1:4319", token: TOKEN });
  assert.equal(link, `http://127.0.0.1:1420/#bridge-daemon=ws%3A%2F%2F127.0.0.1%3A4319&bridge-token=${TOKEN}`);
  const fragment = new URLSearchParams(new URL(link).hash.slice(1));
  assert.equal(fragment.get("bridge-daemon"), "ws://127.0.0.1:4319");
  assert.equal(fragment.get("bridge-token"), TOKEN);
});

test("keeps an app path and drops an existing fragment", () => {
  assert.equal(
    webLink({ app: "https://bridge.example.com/app/#old", daemon: "wss://bridge.example.com/daemon", token: TOKEN }),
    `https://bridge.example.com/app/#bridge-daemon=wss%3A%2F%2Fbridge.example.com%2Fdaemon&bridge-token=${TOKEN}`,
  );
});

test("refuses a daemon URL that is not a WebSocket", () => {
  assert.throws(() => webLink({ daemon: "http://127.0.0.1:4319", token: TOKEN }), /ws:\/\/ or wss:\/\//);
});

test("reads and validates the token file", () => {
  const good = dataDir(`${TOKEN}\n`);
  const bad = dataDir("not-a-token");
  const missing = dataDir();
  try {
    assert.equal(readToken(good), TOKEN);
    assert.throws(() => readToken(bad), /64-character hex token/);
    assert.throws(() => readToken(missing), /no daemon\.token in/);
  } finally {
    for (const dir of [good, bad, missing]) rmSync(dir, { recursive: true, force: true });
  }
});

test("parses flags, the bun separator, and the environment fallback", () => {
  assert.deepEqual(parseArgs(["--", "--data-dir", "/d", "--daemon", "wss://x"], {}), { dataDir: "/d", daemon: "wss://x", app: "http://127.0.0.1:1420" });
  assert.equal(parseArgs([], { BRIDGE_DATA_DIR: "/env" }).dataDir, "/env");
  assert.throws(() => parseArgs([], {}), /--data-dir is required/);
  assert.throws(() => parseArgs(["--nope", "1"], {}), /unknown argument/);
});

test("prints the link on stdout and fails clearly without a token", () => {
  const good = dataDir(TOKEN);
  const missing = dataDir();
  try {
    const ok = spawnSync(process.execPath, [script, "--data-dir", good], { encoding: "utf8" });
    assert.equal(ok.status, 0);
    assert.match(ok.stdout, new RegExp(`bridge-token=${TOKEN}`));
    assert.match(ok.stderr, /--allowed-origin http:\/\/127\.0\.0\.1:1420/);
    const failed = spawnSync(process.execPath, [script, "--data-dir", missing], { encoding: "utf8" });
    assert.equal(failed.status, 1);
    assert.equal(failed.stdout, "");
    assert.match(failed.stderr, /web-link: no daemon\.token/);
  } finally {
    for (const dir of [good, missing]) rmSync(dir, { recursive: true, force: true });
  }
});
