import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, cpSync, writeFileSync, readFileSync, chmodSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../../", import.meta.url));

test("desktop development prepares the real voice helper before starting Tauri", t => {
  const fixture = mkdtempSync(join(tmpdir(), "bridge-tauri-dev-"));
  t.after(() => rmSync(fixture, { recursive: true, force: true }));
  mkdirSync(join(fixture, "scripts"));
  mkdirSync(join(fixture, "node_modules/.bin"), { recursive: true });
  const bin = join(fixture, "bin");
  mkdirSync(bin);
  cpSync(join(root, "scripts/tauri.sh"), join(fixture, "scripts/tauri.sh"));
  const log = join(fixture, "calls");
  for (const [path, label] of [[join(bin, "bun"), "bun"], [join(fixture, "node_modules/.bin/tauri"), "tauri"]]) {
    writeFileSync(path, `#!/bin/sh\nprintf '%s\\n' "${label} $*" >> "$BRIDGE_DEV_TEST_LOG"\n`);
    chmodSync(path, 0o755);
  }
  const result = spawnSync("/bin/sh", [join(fixture, "scripts/tauri.sh"), "dev", "--no-watch"], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, BRIDGE_DEV_TEST_LOG: log }, encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), [
    "bun run prepare:browser-host:dev",
    "bun run prepare:daemon:dev",
    "bun run prepare:voice-helper",
    "tauri dev --no-watch",
  ]);
});
