import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, mkdirSync, rmSync, cpSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../../", import.meta.url));
const credentialNames = ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_KEY", "APPLE_API_KEY_PATH", "APPLE_API_ISSUER", "APPLE_SIGNING_IDENTITY", "BRIDGE_RELEASE_ENV"];
function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-release-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, "bin"));
  const env = { ...process.env, PATH: join(dir, "bin") + ":" + process.env.PATH };
  for (const name of credentialNames) delete env[name];
  return { dir, env };
}
function executable(path, body) {
  writeFileSync(path, body);
  chmodSync(path, 0o755);
}
function common(script, env) {
  return spawnSync("/bin/sh", ["-eu", "-c", '. "$1"; ' + script, "test", join(root, "scripts/release-common.sh")], { env, encoding: "utf8" });
}

test("release env file exports plain assignments to child commands", (t) => {
  const { dir, env } = fixture(t);
  const file = join(dir, "env");
  writeFileSync(file, "APPLE_SIGNING_IDENTITY='Developer ID Application: Test (TESTTEAM)'\nAPPLE_ID=example@test.invalid\n");
  const out = common(`release_load_env; node -e 'if(process.env.APPLE_ID !== "example@test.invalid") process.exit(1)'`, { ...env, BRIDGE_RELEASE_ENV: file });
  assert.equal(out.status, 0, out.stderr);
});

test("notarization validates complete credentials and supports both key types and Apple ID", (t) => {
  const { dir, env } = fixture(t);
  const key = join(dir, "AuthKey.p8");
  writeFileSync(key, "test fixture only");
  for (const extra of [
    {},
    { APPLE_ID: "example@test.invalid" },
    { APPLE_API_KEY: "KEY" },
    { APPLE_API_KEY_PATH: key },
    { APPLE_API_ISSUER: "ISSUER" },
    { APPLE_API_KEY: "KEY", APPLE_API_KEY_PATH: join(dir, "missing") },
  ]) {
    assert.notEqual(common("release_require_credentials", { ...env, ...extra }).status, 0, JSON.stringify(extra));
  }
  for (const extra of [
    { APPLE_API_KEY: "KEY", APPLE_API_KEY_PATH: key },
    { APPLE_API_KEY: "KEY", APPLE_API_KEY_PATH: key, APPLE_API_ISSUER: "ISSUER" },
    { APPLE_ID: "example@test.invalid", APPLE_PASSWORD: "FAKE-PASSWORD", APPLE_TEAM_ID: "TEAM" },
  ]) {
    const out = common("release_require_credentials", { ...env, ...extra });
    assert.equal(out.status, 0, out.stderr);
  }
});

test("notarytool exit zero is insufficient unless the result is Accepted", (t) => {
  const { dir, env } = fixture(t);
  executable(join(dir, "bin/xcrun"), '#!/usr/bin/env node\nrequire("fs").writeFileSync(process.env.ARGUMENTS, JSON.stringify(process.argv.slice(2))); console.log(JSON.stringify({id:"fixture-id",status:process.env.NOTARY_STATUS}));\n');
  const argsFile = join(dir, "args");
  const appleEnv = { ...env, APPLE_ID: "example@test.invalid", APPLE_PASSWORD: "FAKE-PASSWORD", APPLE_TEAM_ID: "TEAM", ARGUMENTS: argsFile };
  const script = 'release_require_credentials; release_notarize "$SUBMISSION" "$RESULT"';
  for (const status of ["Invalid", "In Progress", "Accepted"]) {
    const out = common(script, { ...appleEnv, NOTARY_STATUS: status, SUBMISSION: join(dir, "test.dmg"), RESULT: join(dir, "result.json") });
    assert.equal(out.status === 0, status === "Accepted", out.stderr);
  }
  const args = JSON.parse(readFileSync(argsFile));
  assert.deepEqual(args.slice(3, 9), ["--apple-id", "example@test.invalid", "--password", "FAKE-PASSWORD", "--team-id", "TEAM"]);
  assert.ok(!args.includes("--key"));
});

test("API notary invocation includes issuer only for Team keys", (t) => {
  const { dir, env } = fixture(t);
  executable(join(dir, "bin/xcrun"), '#!/usr/bin/env node\nrequire("fs").writeFileSync(process.env.ARGUMENTS, JSON.stringify(process.argv.slice(2))); console.log(JSON.stringify({id:"fixture-id",status:"Accepted"}));\n');
  const key = join(dir, "AuthKey.p8"), argsFile = join(dir, "args");
  writeFileSync(key, "fixture");
  for (const issuer of [undefined, "ISSUER"]) {
    const extra = issuer ? { APPLE_API_ISSUER: issuer } : {};
    const out = common('release_require_credentials; release_notarize "$SUBMISSION" "$RESULT"', {
      ...env, ...extra, APPLE_API_KEY: "KEY", APPLE_API_KEY_PATH: key, ARGUMENTS: argsFile, SUBMISSION: join(dir, "test.dmg"), RESULT: join(dir, "result.json"),
    });
    assert.equal(out.status, 0, out.stderr);
    const args = JSON.parse(readFileSync(argsFile));
    assert.ok(args.includes("--key"));
    assert.equal(args.includes("--issuer"), Boolean(issuer));
    assert.ok(!args.includes("--apple-id"));
  }
});

test("release identity must be a usable Developer ID identity in the keychain", (t) => {
  const { dir, env } = fixture(t);
  executable(join(dir, "bin/security"), '#!/bin/sh\nprintf \'%s\\n\' \' 1) 0123456789 "Developer ID Application: Test (TESTTEAM)"\'\n');
  const ok = common('release_require_identity; test -n "$APPLE_SIGNING_IDENTITY"', env);
  assert.equal(ok.status, 0, ok.stderr);
  for (const identity of ["-", "Apple Development: Test", "Developer ID Application: Missing (TEAM)"]) {
    assert.notEqual(common("release_require_identity", { ...env, APPLE_SIGNING_IDENTITY: identity }).status, 0);
  }
});

test("sidecar staging invalidates changed locks and keeps the last complete tree on install failure", (t) => {
  const { dir, env } = fixture(t);
  const script = join(dir, "scripts/prepare-claude-sidecar.sh");
  mkdirSync(dirname(script));
  cpSync(join(root, "scripts/prepare-claude-sidecar.sh"), script);
  const src = join(dir, "sidecar/claude-agent");
  mkdirSync(src, { recursive: true });
  for (const name of ["index.mjs", "briefing.mjs", "input.mjs", "options.mjs"]) writeFileSync(join(src, name), "export {};\n");
  writeFileSync(join(src, "package.json"), '{"dependencies":{"@anthropic-ai/claude-agent-sdk":"1.0.0"}}');
  writeFileSync(join(src, "package-lock.json"), '{"fixtureLock":1}');
  const calls = join(dir, "npm-calls");
  executable(join(dir, "bin/npm"), `#!/usr/bin/env node
const fs = require("fs"), path = require("path"), args = process.argv.slice(2);
fs.appendFileSync(process.env.NPM_CALLS, args[0] + "\\n");
if (args[0] === "ci") {
  if (process.env.FAIL_INSTALL) process.exit(1);
  const sdk = path.join(args[args.indexOf("--prefix") + 1], "node_modules/@anthropic-ai/claude-agent-sdk");
  fs.mkdirSync(sdk, {recursive:true});
  fs.writeFileSync(path.join(sdk,"package.json"), '{"version":"1.0.0"}');
}
`);
  const run = (extra = {}) => spawnSync("/bin/sh", [script], { env: { ...env, NPM_CALLS: calls, ...extra }, encoding: "utf8" });
  const ciCount = () => readFileSync(calls, "utf8").split("\n").filter(x => x === "ci").length;
  const dest = join(dir, "src-tauri/resources/sidecar/claude-agent");
  let out = run();
  assert.equal(out.status, 0, out.stderr);
  assert.equal(ciCount(), 1);
  writeFileSync(join(src, "index.mjs"), "export const changed = true;\n");
  out = run();
  assert.equal(out.status, 0, out.stderr);
  assert.equal(ciCount(), 1, "unchanged dependency lock should reuse the tree");
  assert.equal(readFileSync(join(dest, "index.mjs"), "utf8"), "export const changed = true;\n");
  writeFileSync(join(src, "package-lock.json"), '{"fixtureLock":2}');
  out = run({ FAIL_INSTALL: "1" });
  assert.notEqual(out.status, 0);
  assert.equal(readFileSync(join(dest, "package-lock.json"), "utf8"), '{"fixtureLock":1}', "failed installs retain the last complete stage");
  out = run();
  assert.equal(out.status, 0, out.stderr);
  assert.equal(readFileSync(join(dest, "package-lock.json"), "utf8"), '{"fixtureLock":2}');
  assert.equal(ciCount(), 3);
  writeFileSync(join(dest, ".bridge-stage-platform"), "other-platform");
  out = run();
  assert.equal(out.status, 0, out.stderr);
  assert.equal(ciCount(), 4, "platform changes must not reuse optional native dependencies");
});

test("a successful build routed to a different Cargo target cannot release a stale default app", (t) => {
  const { dir, env } = fixture(t);
  const scripts = join(dir, "scripts");
  mkdirSync(scripts);
  for (const name of ["release-dmg.sh", "release-common.sh"]) cpSync(join(root, "scripts", name), join(scripts, name));
  const app = join(dir, "src-tauri/target/release/bundle/macos/Bridge.app");
  mkdirSync(app, { recursive: true });
  writeFileSync(join(app, "build-marker"), "previous build");
  executable(join(dir, "bin/security"), '#!/bin/sh\nprintf \'%s\\n\' \' 1) 0123456789 "Developer ID Application: Test (TESTTEAM)"\'\n');
  executable(join(dir, "bin/npm"), "#!/bin/sh\nexit 0\n");
  executable(join(dir, "bin/node"), `#!/bin/sh
if [ "$1" = -p ]; then printf '%s\\n' 0.5.2; exit 0; fi
mkdir -p "$ALTERNATE_APP"
printf '%s' 'new target build' > "$ALTERNATE_APP/build-marker"
`);
  executable(join(scripts, "verify-macos-app.sh"), '#!/bin/sh\nprintf verify >> "$EVENTS"\n');
  executable(join(dir, "bin/xcrun"), '#!/bin/sh\nprintf notary >> "$EVENTS"\nexit 1\n');
  const events = join(dir, "events");
  writeFileSync(events, "");
  const out = spawnSync("/bin/sh", [join(scripts, "release-dmg.sh")], {
    env: { ...env, BRIDGE_RELEASE_ENV: join(dir, "no-release-env"), APPLE_ID: "example@test.invalid", APPLE_PASSWORD: "FIXTURE", APPLE_TEAM_ID: "TESTTEAM", ALTERNATE_APP: join(dir, "other-target/Bridge.app"), EVENTS: events, TMPDIR: dir }, encoding: "utf8",
  });
  assert.notEqual(out.status, 0);
  assert.match(out.stderr, /did not produce the expected Bridge.app/);
  assert.equal(readFileSync(events, "utf8"), "", "stale output must not reach verification or notarization");
  assert.equal(readFileSync(join(app, "build-marker"), "utf8"), "previous build", "failed builds restore the previous development artifact");
});

test("GitHub release remains a draft if uploading its verified assets fails", (t) => {
  const { dir, env } = fixture(t);
  const workflow = readFileSync(join(root, ".github/workflows/release-macos.yml"), "utf8");
  const step = workflow.split("      - name: Publish verified tagged release\n")[1].split("\n      - name:")[0];
  const script = step.split("        run: |\n")[1].split("\n").map(line => line.replace(/^          /, "")).join("\n");
  mkdirSync(join(dir, "src-tauri"));
  writeFileSync(join(dir, "src-tauri/tauri.conf.json"), '{"version":"0.5.2"}');
  const events = join(dir, "events");
  executable(join(dir, "bin/gh"), `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
fs.appendFileSync(process.env.EVENTS, JSON.stringify(args) + "\\n");
if (args[1] === "create" && process.env.FAIL_UPLOAD) process.exit(1);
`);
  for (const fail of [true, false]) {
    writeFileSync(events, "");
    const out = spawnSync("/bin/bash", ["--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script], {
      cwd: dir, env: { ...env, EVENTS: events, RUNNER_TEMP: dir, GITHUB_REF_NAME: "v0.5.2", FAIL_UPLOAD: fail ? "1" : "" }, encoding: "utf8",
    });
    const calls = readFileSync(events, "utf8").trim().split("\n").map(line => JSON.parse(line));
    assert.ok(calls[0].includes("--draft"), "asset uploads must start in a draft");
    assert.equal(out.status === 0, !fail, out.stderr);
    if (fail) assert.equal(calls.length, 1, "upload failures must not reach publication");
    else assert.deepEqual(calls[1], ["release", "edit", "v0.5.2", "--draft=false"]);
  }
});
