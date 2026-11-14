import { test } from "node:test";
import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../../", import.meta.url));
const wrapper = join(root, "scripts/linuxdeploy-patchelf.sh");
const sdkBinary = "/AppDir/usr/lib/Bridge/sidecar/claude-agent/node_modules/@anthropic-ai/claude-agent-sdk-linux-x64/claude";

// A stand-in patchelf that records how it was called.
function fakePatchelf(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-patchelf-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const log = join(dir, "calls");
  const bin = join(dir, "patchelf");
  writeFileSync(bin, `#!/bin/sh\nprintf '%s\\n' "$*" >> "${log}"\n`);
  chmodSync(bin, 0o755);
  const run = (...args) => spawnSync("sh", [wrapper, ...args], { env: { ...process.env, BRIDGE_REAL_PATCHELF: bin }, encoding: "utf8" });
  const calls = () => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n") : []);
  return { run, calls };
}

test("never lets linuxdeploy rewrite the Claude SDK's native binary", (t) => {
  const patchelf = fakePatchelf(t);
  for (const variant of ["linux-x64", "linux-x64-musl", "linux-arm64"]) {
    const result = patchelf.run("--set-rpath", "$ORIGIN", sdkBinary.replace("linux-x64", variant));
    assert.equal(result.status, 0, result.stderr);
  }
  assert.equal(patchelf.run("--remove-rpath", sdkBinary).status, 0);
  assert.deepEqual(patchelf.calls(), []);
});

test("still answers read-only questions about it", (t) => {
  const patchelf = fakePatchelf(t);
  assert.equal(patchelf.run("--print-rpath", sdkBinary).status, 0);
  assert.deepEqual(patchelf.calls(), [`--print-rpath ${sdkBinary}`]);
});

test("passes every other file through unchanged", (t) => {
  const patchelf = fakePatchelf(t);
  const library = "/AppDir/usr/lib/libgtk-3.so.0";
  assert.equal(patchelf.run("--set-rpath", "$ORIGIN", library).status, 0);
  // A file merely named claude elsewhere is not the SDK binary.
  assert.equal(patchelf.run("--set-rpath", "$ORIGIN", "/AppDir/usr/bin/claude").status, 0);
  assert.deepEqual(patchelf.calls(), [`--set-rpath $ORIGIN ${library}`, "--set-rpath $ORIGIN /AppDir/usr/bin/claude"]);
});

test("tauri.sh routes linuxdeploy's patchelf through the wrapper on Linux builds only", () => {
  const script = readFileSync(join(root, "scripts/tauri.sh"), "utf8");
  assert.match(script, /if \[ "\$\{1:-\}" = "build" \] && \[ "\$\(uname -s\)" = "Linux" \]; then/);
  assert.match(script, /BRIDGE_REAL_PATCHELF=\$\{PATCHELF:-patchelf\}/);
  assert.match(script, /PATCHELF="\$project_root\/scripts\/linuxdeploy-patchelf\.sh"/);
  assert.match(script, /export BRIDGE_REAL_PATCHELF PATCHELF/);
});
