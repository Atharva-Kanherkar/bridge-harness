import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, readFileSync, writeFileSync, cpSync, chmodSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../../", import.meta.url));
function bundleFixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-bundle-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const bin = join(dir, "bin"), app = join(dir, "Bridge Test.app"), contents = join(app, "Contents");
  mkdirSync(bin);
  mkdirSync(join(contents, "Resources"), { recursive: true });
  mkdirSync(join(contents, "MacOS"));
  const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json")));
  writeFileSync(join(contents, "Info.plist"), `<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>${config.version}</string><key>CFBundleIdentifier</key><string>${config.identifier}</string><key>CFBundleIconFile</key><string>icon.icns</string></dict></plist>`);
  cpSync(join(root, "src-tauri/icons/icon.icns"), join(contents, "Resources/icon.icns"));
  const sidecar = join(contents, "Resources/sidecar/claude-agent");
  mkdirSync(sidecar, { recursive: true });
  for (const name of ["index.mjs", "briefing.mjs", "input.mjs", "options.mjs", "package.json", "package-lock.json"])
    cpSync(join(root, "sidecar/claude-agent", name), join(sidecar, name));
  const sdk = join(sidecar, "node_modules/@anthropic-ai/claude-agent-sdk");
  mkdirSync(sdk, { recursive: true });
  const lock = JSON.parse(readFileSync(join(sidecar, "package-lock.json")));
  const sdkVersion = lock.packages["node_modules/@anthropic-ai/claude-agent-sdk"].version;
  writeFileSync(join(sdk, "package.json"), JSON.stringify({ version: sdkVersion }));
  const nativeDir = join(sidecar, "node_modules/@anthropic-ai/claude-agent-sdk-darwin-" + process.arch);
  const native = join(nativeDir, "claude");
  mkdirSync(nativeDir);
  writeFileSync(join(nativeDir, "package.json"), JSON.stringify({ version: sdkVersion }));
  writeFileSync(native, "native fixture");
  chmodSync(native, 0o755);
  for (const name of ["bridged", "bridge-browser-host"]) {
    writeFileSync(join(contents, "MacOS", name), "fixture");
    chmodSync(join(contents, "MacOS", name), 0o755);
  }
  const signature = join(dir, "signature"), entitlements = join(dir, "entitlements.plist");
  const nativeSignature = join(dir, "native-signature"), nativeEntitlements = join(dir, "native-entitlements.plist");
  const validSignature = "Authority=Developer ID Application: Fixture (TESTTEAM)\nTeamIdentifier=TESTTEAM\nCodeDirectory v=20500 size=123 flags=0x10000(runtime) hashes=1\n";
  const validNativeSignature = validSignature.replaceAll("TESTTEAM", "VENDORTEAM").replace("Fixture", "Vendor");
  writeFileSync(signature, validSignature);
  writeFileSync(nativeSignature, validNativeSignature);
  cpSync(join(root, "src-tauri/entitlements.plist"), entitlements);
  cpSync(entitlements, nativeEntitlements);
  writeFileSync(join(bin, "codesign"), `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
const native = args.at(-1).endsWith("/claude");
if (args.includes("--verify")) process.exit((native ? process.env.INVALID_NATIVE_SIGNATURE : process.env.INVALID_SIGNATURE) ? 1 : 0);
if (args.includes("--entitlements")) {
  if (!args.includes("--xml")) process.stdout.write("[Dict]");
  else process.stdout.write(fs.readFileSync(native ? process.env.NATIVE_ENTITLEMENTS : process.env.ENTITLEMENTS));
} else process.stderr.write(fs.readFileSync(native ? process.env.NATIVE_SIGNATURE : process.env.SIGNATURE));
`);
  chmodSync(join(bin, "codesign"), 0o755);
  const run = (extra = {}) => spawnSync("/bin/sh", [join(root, "scripts/verify-macos-app.sh"), app], {
    env: { ...process.env, PATH: bin + ":" + process.env.PATH, SIGNATURE: signature, ENTITLEMENTS: entitlements, NATIVE_SIGNATURE: nativeSignature, NATIVE_ENTITLEMENTS: nativeEntitlements, ...extra }, encoding: "utf8",
  });
  return { run, signature, validSignature, entitlements, sidecar, sdk, sdkVersion, contents, nativeDir, native, nativeSignature, validNativeSignature, nativeEntitlements };
}

test("app release gate rejects ad-hoc signing, false JIT entitlements, stale resources, and missing native binaries", (t) => {
  const { run, signature, validSignature, entitlements, sidecar, sdk, sdkVersion, contents } = bundleFixture(t);
  let out = run();
  assert.equal(out.status, 0, out.stderr);
  out = run({ INVALID_SIGNATURE: "1" });
  assert.notEqual(out.status, 0, "codesign verification failure must fail the release");

  for (const invalid of [
    validSignature.replace("Authority=Developer ID Application: Fixture (TESTTEAM)", "Signature=adhoc"),
    validSignature.replace("TeamIdentifier=TESTTEAM", "TeamIdentifier=not set"),
    validSignature.replace("flags=0x10000(runtime)", "flags=0x0(none)"),
  ]) {
    writeFileSync(signature, invalid);
    assert.notEqual(run().status, 0);
  }
  writeFileSync(signature, validSignature);
  const validEntitlements = readFileSync(entitlements, "utf8");
  writeFileSync(entitlements, validEntitlements.replace(/(<key>com.apple.security.cs.allow-jit<\/key>\s*)<true\/>/, "$1<false/>"));
  assert.notEqual(run().status, 0, "the entitlement key being present with false must fail");
  writeFileSync(entitlements, validEntitlements);

  writeFileSync(join(sidecar, "index.mjs"), "// stale sidecar");
  assert.notEqual(run().status, 0);
  cpSync(join(root, "sidecar/claude-agent/index.mjs"), join(sidecar, "index.mjs"));
  writeFileSync(join(sdk, "package.json"), '{"version":"0.0.0"}');
  assert.notEqual(run().status, 0, "the shipped SDK must match the shipped lock");
  writeFileSync(join(sdk, "package.json"), JSON.stringify({ version: sdkVersion }));
  rmSync(join(contents, "MacOS/bridged"));
  assert.notEqual(run().status, 0, "daemon must be an executable in the app");
});

test("native Claude SDK must be present, executable, locked, and signed with JIT while allowing its vendor Team ID", (t) => {
  const { run, nativeDir, native, nativeSignature, validNativeSignature, nativeEntitlements, sdkVersion } = bundleFixture(t);
  let out = run();
  assert.equal(out.status, 0, "upstream vendor Team ID must be accepted: " + out.stderr);
  out = run({ INVALID_NATIVE_SIGNATURE: "1" });
  assert.notEqual(out.status, 0, "tampered upstream native binaries must fail");
  for (const invalid of [
    validNativeSignature.replace("Authority=Developer ID Application: Vendor (VENDORTEAM)", "Signature=adhoc"),
    validNativeSignature.replace("flags=0x10000(runtime)", "flags=0x0(none)"),
  ]) {
    writeFileSync(nativeSignature, invalid);
    assert.notEqual(run().status, 0);
  }
  writeFileSync(nativeSignature, validNativeSignature);
  const originalEntitlements = readFileSync(nativeEntitlements, "utf8");
  writeFileSync(nativeEntitlements, originalEntitlements.replace(/(<key>com.apple.security.cs.allow-jit<\/key>\s*)<true\/>/, "$1<false/>"));
  assert.notEqual(run().status, 0, "native JIT must remain enabled after packaging");
  writeFileSync(nativeEntitlements, originalEntitlements);
  writeFileSync(join(nativeDir, "package.json"), '{"version":"0.0.0"}');
  assert.notEqual(run().status, 0, "native package must match its SDK and lock");
  writeFileSync(join(nativeDir, "package.json"), JSON.stringify({ version: sdkVersion }));
  chmodSync(native, 0o644);
  assert.notEqual(run().status, 0, "native executable permissions must survive staging");
  rmSync(native);
  out = run();
  assert.notEqual(out.status, 0, "omitted optional native package must fail despite the JS SDK being installed");
  assert.match(out.stderr, /missing real executable Claude SDK binary/);
  symlinkSync("/bin/sh", native);
  assert.notEqual(run().status, 0, "native executable must be a real file inside the bundle");
});
