import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, cpSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { nextVersion, syncVersions, versionFiles, publish } from "../ci-release.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-ci-release-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}
function versionFixture(t) {
  const dir = fixture(t);
  for (const file of versionFiles) {
    mkdirSync(dirname(join(dir, file)), { recursive: true });
    cpSync(join(root, file), join(dir, file));
  }
  return dir;
}

test("versions advance numerically beyond all tags and honor deliberate larger versions", () => {
  assert.equal(nextVersion("0.5.4", ["v0.5.4", "v0.5.9", "v0.5.10"]), "0.5.11");
  assert.equal(nextVersion("0.5.5", ["v0.5.4", "v0.5.5"]), "0.5.6");
  assert.equal(nextVersion("0.6.0", ["v0.5.99", "v99.0.0-beta.1"]), "0.6.0");
  assert.equal(nextVersion("0.5.5", ["v0.5.4"]), "0.5.5");
  assert.equal(nextVersion("0.5.5", []), "0.5.5");
  assert.throws(() => nextVersion("0.5.5-rc.1", []));
});

test("all app versions and workspace locks change without altering external dependencies", (t) => {
  const dir = versionFixture(t);
  const json = file => JSON.parse(readFileSync(join(dir, file)));
  const before = json("sidecar/claude-agent/package-lock.json");
  const cargoBefore = readFileSync(join(dir, "src-tauri/Cargo.lock"), "utf8");
  syncVersions(dir, "0.5.123");
  for (const file of ["package.json", "src-tauri/tauri.conf.json", "sidecar/claude-agent/package.json", "browser-extension/manifest.json", "safari-extension/Resources/manifest.json"]) {
    assert.equal(json(file).version, "0.5.123", file);
  }
  const after = json("sidecar/claude-agent/package-lock.json");
  assert.equal(after.version, "0.5.123");
  assert.equal(after.packages[""].version, "0.5.123");
  for (const name of Object.keys(before.packages).filter(Boolean)) assert.deepEqual(after.packages[name], before.packages[name]);
  const external = text => text.split("[[package]]").filter(block => block.includes("\nsource = "));
  assert.deepEqual(external(readFileSync(join(dir, "src-tauri/Cargo.lock"), "utf8")), external(cargoBefore));
  assert.match(readFileSync(join(dir, "bun.lock"), "utf8"), /"sidecar\/claude-agent": \{\s*"name": "bridge-claude-agent-sidecar",\s*"version": "0.5.123"/);
  assert.match(readFileSync(join(dir, "src-tauri/Cargo.toml"), "utf8"), /\[workspace.package\]\s*version = "0.5.123"/);
  for (const name of ["bridge-deck", "bridge-core", "bridge-client", "bridge-protocol", "bridged"]) {
    assert.ok(readFileSync(join(dir, "src-tauri/Cargo.lock"), "utf8").includes(`name = "${name}"\nversion = "0.5.123"`));
  }
});

test("unexpected lock structure fails before any version file is modified", (t) => {
  const dir = versionFixture(t);
  const before = readFileSync(join(dir, "package.json"), "utf8");
  writeFileSync(join(dir, "bun.lock"), "{}");
  assert.throws(() => syncVersions(dir, "0.5.123"), /bun.lock/);
  assert.equal(readFileSync(join(dir, "package.json"), "utf8"), before);
});

function publisherFixture(t) {
  const dir = fixture(t), calls = [], gitCalls = [];
  const dmg = join(dir, "Bridge_0.5.6_aarch64.dmg");
  writeFileSync(dmg, "fixture DMG");
  writeFileSync(`${dmg}.sha256`, `${createHash("sha256").update("fixture DMG").digest("hex")}  Bridge_0.5.6_aarch64.dmg\n`);
  const source = "1".repeat(40), commit = "2".repeat(40);
  return {
    calls, gitCalls,
    args: { source, commit, dmg, tag: "v0.5.6", releaseList: [],
      command: (...args) => { calls.push(args); return ""; },
      gitCommand: (...args) => {
        gitCalls.push(args);
        if (args[0] === "rev-parse") return commit;
        if (args[0] === "show") return source;
        return "";
      },
    },
  };
}

test("failed asset upload stays a draft and never publishes", (t) => {
  const { args, calls } = publisherFixture(t);
  const record = args.command;
  args.command = (...call) => { record(...call); if (call[1] === "upload") throw new Error("upload failed"); };
  assert.throws(() => publish(args), /upload failed/);
  assert.ok(calls[0].includes("--draft"));
  assert.deepEqual(calls.map(call => call[1]), ["create", "upload"]);
});

test("draft retries finish both assets, but older retries do not become Latest", (t) => {
  const { args, calls } = publisherFixture(t);
  args.releaseList = [{ tag_name: "v0.5.6", draft: true }, { tag_name: "v0.5.7", draft: false }];
  publish(args);
  assert.deepEqual(calls.map(call => call[1]), ["upload", "edit"]);
  assert.ok(calls[0].includes(`${args.dmg}.sha256`));
  assert.deepEqual(calls[1], ["release", "edit", "v0.5.6", "--draft=false", "--latest=false"]);
});

test("successful new releases publish only after upload; public releases are immutable on rerun", (t) => {
  const { args, calls, gitCalls } = publisherFixture(t);
  publish(args);
  assert.deepEqual(calls.map(call => call[1]), ["create", "upload", "edit"]);
  assert.ok(calls[2].includes("--latest=true"));
  assert.ok(gitCalls.some(call => call[0] === "push" && call[2] === "refs/tags/v0.5.6"));
  calls.length = gitCalls.length = 0;
  args.releaseList = [{ tag_name: "v0.5.6", draft: false }];
  publish(args);
  assert.equal(calls.length, 0);
  assert.ok(!gitCalls.some(call => call[0] === "push"));
});

test("checksum mismatches and tag collisions fail before any release mutation", (t) => {
  const { args, calls } = publisherFixture(t);
  const original = readFileSync(args.dmg);
  writeFileSync(args.dmg, "changed after verification");
  assert.throws(() => publish(args), /checksum mismatch/);
  writeFileSync(args.dmg, original);
  const normalGit = args.gitCommand;
  args.gitCommand = (...call) => {
    if (call[0] === "tag" && call[1] === "--list") return args.tag;
    if (call[0] === "rev-parse" && call[1] !== "HEAD") return "3".repeat(40);
    return normalGit(...call);
  };
  assert.throws(() => publish(args), /another commit/);
  assert.equal(calls.length, 0);
});

test("preparation tags the exact versioned source and resumes the same merge without changing main", (t) => {
  const dir = versionFixture(t), remote = fixture(t), bin = join(dir, "bin");
  const git = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "--initial-branch=main");
  git("config", "user.name", "Test"); git("config", "user.email", "test@example.invalid");
  git("add", "."); git("commit", "-m", "fixture source");
  git("init", "--bare", remote); git("remote", "add", "origin", remote);
  git("push", "origin", "main");
  const source = git("rev-parse", "HEAD");
  mkdirSync(bin);
  writeFileSync(join(bin, "gh"), '#!/bin/sh\nif [ -n "$FAIL_API" ]; then exit 1; fi\nprintf \'%s\\n\' "${RELEASES_JSON:-[[]]}"\n', { mode: 0o755 });
  const outFile = join(remote, "outputs");
  // Keep fixture helper files out of the checkout's tracked/untracked state.
  writeFileSync(join(dir, ".git/info/exclude"), "bin/\n");
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, GITHUB_REF: "refs/heads/main", GITHUB_SHA: source, GITHUB_REPOSITORY: "fixture/repo", GITHUB_OUTPUT: outFile };
  const prepare = extra => spawnSync(process.execPath, [join(root, "scripts/ci-release.mjs"), "prepare"], { cwd: dir, env: { ...env, ...extra }, encoding: "utf8" });
  const failed = prepare({ FAIL_API: "1" });
  assert.notEqual(failed.status, 0);
  assert.equal(git("rev-parse", "HEAD"), source);
  let result = prepare();
  assert.equal(result.status, 0, result.stderr);
  const output = () => Object.fromEntries(readFileSync(outFile, "utf8").trim().split("\n").map(line => line.split("=")));
  const first = output();
  assert.equal(first.published, "false");
  assert.equal(git("rev-parse", "main"), source);
  assert.equal(git("rev-parse", "HEAD^"), source);
  assert.equal(git("status", "--porcelain"), "");
  git("tag", first.tag); git("push", "origin", `refs/tags/${first.tag}`);
  git("checkout", "--detach", source);
  result = prepare({ RELEASES_JSON: JSON.stringify([[{ tag_name: first.tag, draft: false }]]) });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(output(), { ...first, published: "true" });
  assert.equal(git("rev-parse", "HEAD"), first.commit);
  result = prepare({ GITHUB_REF: "refs/heads/feature" });
  assert.notEqual(result.status, 0, "manual branch runs must not publish");
});
