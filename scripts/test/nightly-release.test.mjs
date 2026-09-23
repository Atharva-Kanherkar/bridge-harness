import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, mkdirSync, rmSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import {
  conventionalType,
  groupForTitle,
  istDayBounds,
  mainSnapshotBefore,
  planNightly,
  renderNightlyNotes,
  runPlan,
  searchQuery,
  selectPullRequests,
  shippedIstDate,
} from "../nightly-release.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const sha = "0123456789abcdef0123456789abcdef01234567";

function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-nightly-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, "bin"));
  const env = { ...process.env, PATH: `${join(dir, "bin")}:${process.env.PATH}` };
  return { dir, env };
}
function executable(path, body) {
  writeFileSync(path, body);
  chmodSync(path, 0o755);
}
function namedStep(workflow, name) {
  const lines = workflow.split("\n");
  const start = lines.findIndex((line) => line === `      - name: ${name}`);
  assert.ok(start !== -1, name);
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (lines[i].startsWith("      - name:") || lines[i].startsWith("      #")) {
      end = i;
      break;
    }
  }
  return lines.slice(start, end).join("\n");
}
function runScript(workflow, name) {
  const step = namedStep(workflow, name);
  const script = step.split("        run: |\n")[1];
  assert.ok(script, name);
  return script.split("\n").map((line) => line.replace(/^          /, "")).join("\n");
}

test("01:00 IST ships the calendar day that just ended", () => {
  const ranAt = new Date("2026-09-23T19:30:00.000Z");
  assert.equal(shippedIstDate(ranAt), "2026-09-23");
  assert.equal(shippedIstDate(new Date("2026-09-24T18:29:59.000Z")), "2026-09-23");
  // 18:30 UTC is 00:00 IST the next calendar day, so the day that just ended is still the 23rd.
  assert.equal(shippedIstDate(new Date("2026-09-23T18:30:00.000Z")), "2026-09-23");
  assert.equal(shippedIstDate(new Date("2026-09-23T18:29:59.000Z")), "2026-09-22");
});

test("IST day bounds are 18:30 UTC exclusive at the end", () => {
  const { start, end } = istDayBounds("2026-09-23");
  assert.equal(start.toISOString(), "2026-09-22T18:30:00.000Z");
  assert.equal(end.toISOString(), "2026-09-23T18:30:00.000Z");
  assert.equal(mainSnapshotBefore("2026-09-23"), "2026-09-23T19:30:01.000Z");
  assert.throws(() => istDayBounds("2026-02-31"), /real calendar day/);
  assert.throws(() => istDayBounds("2026-9-23"), /YYYY-MM-DD/);
});

test("search covers the UTC dates that contain the IST day", () => {
  assert.equal(
    searchQuery("Atharva-Kanherkar/bridge-harness", "2026-09-23"),
    "repo:Atharva-Kanherkar/bridge-harness is:pr is:merged base:main merged:2026-09-22..2026-09-23",
  );
});

test("conventional commit types map onto the nightly sections", () => {
  assert.equal(groupForTitle("feat: add tabs"), "Features / Improvements");
  assert.equal(groupForTitle("feat(browser)!: replace tabs"), "Features / Improvements");
  assert.equal(groupForTitle("Fix: menu flicker"), "Bug fixes");
  assert.equal(groupForTitle("perf(startup): faster boot"), "Performance");
  assert.equal(groupForTitle("refactor: split release"), "Refactors");
  assert.equal(groupForTitle("docs: nightly notes"), "Docs");
  for (const type of ["chore", "ci", "build", "test", "style"]) {
    assert.equal(groupForTitle(`${type}(repo): tidy`), "Chores & maintenance");
  }
  assert.equal(groupForTitle("revert: drop nightly"), "Other");
  assert.equal(groupForTitle("Release 0.5.10"), "Other");
  assert.equal(conventionalType("feat:add"), "feat");
});

test("notes keep IST-window pulls, group them, and omit empty sections", () => {
  const notes = renderNightlyNotes({
    date: "2026-09-23",
    sha,
    prs: [
      { number: 3, title: "chore: lockfile", url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/3", mergedAt: "2026-09-23T02:00:00.000Z", author: "octocat" },
      { number: 2, title: "fix: menu card", url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/2", mergedAt: "2026-09-23T01:00:00.000Z", author: "bad login" },
      { number: 1, title: "feat: native tabs", url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/1", mergedAt: "2026-09-22T18:30:00.000Z", author: "bridge-dev" },
      { number: 9, title: "feat: next day", url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/9", mergedAt: "2026-09-23T18:30:00.000Z", author: "octocat" },
      { number: 1, title: "feat: native tabs", url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/1", mergedAt: "2026-09-22T18:30:00.000Z", author: "bridge-dev" },
    ],
  });
  assert.match(notes, /Nightly macOS build for 2026-09-23/);
  assert.match(notes, new RegExp(`Built from \`main\` at \`${sha}\``));
  assert.doesNotMatch(notes, /## Performance/);
  assert.doesNotMatch(notes, /next day/);
  const feat = notes.indexOf("## Features / Improvements");
  const fix = notes.indexOf("## Bug fixes");
  const chore = notes.indexOf("## Chores & maintenance");
  assert.ok(feat < fix && fix < chore);
  assert.match(notes, /- feat: native tabs \(\[#1\]\(https:\/\/github.com\/Atharva-Kanherkar\/bridge-harness\/pull\/1\)\) by @bridge-dev/);
  assert.match(notes, /- fix: menu card \(\[#2\]\(https:\/\/github.com\/Atharva-Kanherkar\/bridge-harness\/pull\/2\)\)\n/);
  assert.match(notes, /- chore: lockfile \(\[#3\]\(https:\/\/github.com\/Atharva-Kanherkar\/bridge-harness\/pull\/3\)\) by @octocat/);
  assert.equal(selectPullRequests([{ number: 9, title: "feat: next day", mergedAt: "2026-09-23T18:30:00.000Z" }], "2026-09-23").length, 0);
});

test("plan skips empty days and existing tags without a sha", () => {
  const now = new Date("2026-09-23T19:30:00.000Z");
  const empty = planNightly({ now, prs: [], releaseExists: false, tagExists: false, sha: "" });
  assert.deepEqual(empty, { skip: true, reason: "no-merged-prs", tag: "nightly-2026-09-23", date: "2026-09-23", sha: "", notes: "" });
  const existing = planNightly({ date: "2026-09-23", prs: [{ number: 1, title: "feat: x", mergedAt: "2026-09-23T12:00:00.000Z" }], releaseExists: true, tagExists: false, sha });
  assert.equal(existing.reason, "release-exists");
  assert.equal(existing.skip, true);
  const tagged = planNightly({ date: "2026-09-23", prs: [], releaseExists: false, tagExists: true, sha: "" });
  assert.equal(tagged.reason, "tag-exists");
});

test("plan command writes skip outputs and does not build notes for an empty day", (t) => {
  const { dir, env } = fixture(t);
  executable(join(dir, "bin/gh"), `#!/usr/bin/env node
const fs = require("fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.GH_LOG, JSON.stringify(args) + "\\n");
if (args[0] === "release") {
  process.stderr.write("release not found\\n");
  process.exit(1);
}
if (String(args[1] || "").includes("/git/ref/tags/")) {
  process.stderr.write("gh: Not Found (HTTP 404)\\n");
  process.exit(1);
}
process.stdout.write(JSON.stringify({ total_count: 1, incomplete_results: false, items: [{
  number: 4, title: "feat: outside", html_url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/4",
  user: { login: "octocat" }, pull_request: { merged_at: "2026-09-22T10:00:00Z" },
}]}));
`);
  executable(join(dir, "bin/git"), "#!/bin/sh\nprintf called >&2\nexit 1\n");
  const output = join(dir, "github-output");
  const notes = join(dir, "notes.md");
  writeFileSync(output, "");
  const out = spawnSync(process.execPath, [join(root, "scripts/nightly-release.mjs")], {
    env: {
      ...env,
      GH_LOG: join(dir, "gh.log"),
      GITHUB_OUTPUT: output,
      GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness",
      NIGHTLY_NOW: "2026-09-23T19:30:00.000Z",
      NIGHTLY_NOTES_PATH: notes,
    },
    encoding: "utf8",
  });
  assert.equal(out.status, 0, out.stderr);
  const text = readFileSync(output, "utf8");
  assert.match(text, /^skip=true$/m);
  assert.match(text, /^reason=no-merged-prs$/m);
  assert.match(text, /^tag=nightly-2026-09-23$/m);
  assert.equal(readFileSync(join(dir, "gh.log"), "utf8").includes("git rev-list"), false);
  assert.throws(() => readFileSync(notes, "utf8"));
});

test("plan command renders notes from merged main pulls and resolves the 01:00 IST snapshot", (t) => {
  const { dir, env } = fixture(t);
  executable(join(dir, "bin/gh"), `#!/usr/bin/env node
const args = process.argv.slice(2);
if (args[0] === "release" || String(args[1] || "").includes("/git/ref/tags/")) {
  process.stderr.write("not found\\n");
  process.exit(1);
}
if (process.env.INCOMPLETE) {
  process.stdout.write(JSON.stringify({ total_count: 1, incomplete_results: true, items: [] }));
  process.exit(0);
}
process.stdout.write(JSON.stringify({ total_count: 1, incomplete_results: false, items: [{
  number: 12, title: "ci: nightly dmg", html_url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/12",
  user: { login: "octocat" }, pull_request: { merged_at: "2026-09-23T12:00:00Z" },
}]}));
`);
  executable(join(dir, "bin/git"), `#!/usr/bin/env node
const fs = require("fs");
fs.writeFileSync(process.env.GIT_ARGS, JSON.stringify(process.argv.slice(2)));
if (process.argv.includes("rev-list")) process.stdout.write(process.env.SHA + "\\n");
`);
  const notes = join(dir, "notes.md");
  const out = spawnSync(process.execPath, [join(root, "scripts/nightly-release.mjs")], {
    env: {
      ...env,
      GIT_ARGS: join(dir, "git.json"),
      SHA: sha,
      GITHUB_OUTPUT: join(dir, "out"),
      GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness",
      NIGHTLY_DATE: "2026-09-23",
      NIGHTLY_NOTES_PATH: notes,
    },
    encoding: "utf8",
  });
  assert.equal(out.status, 0, out.stderr);
  assert.match(readFileSync(join(dir, "out"), "utf8"), /skip=false/);
  assert.deepEqual(JSON.parse(readFileSync(join(dir, "git.json"), "utf8")), ["rev-list", "-1", "--before=2026-09-23T19:30:01.000Z", "origin/main"]);
  const body = readFileSync(notes, "utf8");
  assert.match(body, /## Chores & maintenance/);
  assert.match(body, /ci: nightly dmg \(\[#12\]/);
  const bad = spawnSync(process.execPath, [join(root, "scripts/nightly-release.mjs")], {
    env: { ...env, INCOMPLETE: "1", GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness", NIGHTLY_DATE: "2026-09-23", NIGHTLY_NOTES_PATH: notes },
    encoding: "utf8",
  });
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /incomplete results/);
  const invalid = spawnSync(process.execPath, [join(root, "scripts/nightly-release.mjs")], {
    env: { ...env, GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness", NIGHTLY_DATE: "yesterday" },
    encoding: "utf8",
  });
  assert.notEqual(invalid.status, 0);
});

test("existing release or tag short-circuits before the pull request search", () => {
  const calls = [];
  const result = runPlan(
    { NIGHTLY_DATE: "2026-09-23", GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness" },
    {
      releaseExists() { calls.push("release"); return true; },
      tagExists() { calls.push("tag"); return false; },
      fetchPullRequests() { throw new Error("should not search"); },
      resolveSha() { throw new Error("should not resolve"); },
    },
  );
  assert.equal(result.reason, "release-exists");
  assert.deepEqual(calls, ["release"]);
});

test("nightly workflow reuses the stable signing steps and does not publish the updater channel", () => {
  const stable = readFileSync(join(root, ".github/workflows/release-macos.yml"), "utf8");
  const nightly = readFileSync(join(root, ".github/workflows/nightly-macos.yml"), "utf8");
  for (const name of [
    "Install locked dependencies",
    "Import signing identity into a temporary keychain",
    "Build, test, sign, notarize, and verify exact DMG contents",
    "Retain verified DMG and updater artifacts",
    "Remove temporary signing credentials",
  ]) {
    assert.equal(namedStep(nightly, name), namedStep(stable, name), name);
  }
  assert.match(nightly, /cron: "30 19 \* \* \*"/);
  assert.match(nightly, /ref: \$\{\{ needs\.plan\.outputs\.sha \}\}/);
  assert.match(nightly, /needs\.plan\.result == 'success' && needs\.plan\.outputs\.skip == 'false'/);
  assert.match(stable, /tags:\n\s+- "v\*\.\*\.\*"/);
  assert.match(stable, /latest\.json/);
  assert.match(runScript(stable, "Validate release tag and signing inputs"), /v\$version/);
  const validate = runScript(nightly, "Validate signing inputs");
  assert.doesNotMatch(validate, /v\$version/);
  assert.doesNotMatch(validate, /tauri\.conf\.json/);
  const publish = runScript(nightly, "Publish nightly prerelease");
  assert.match(publish, /--prerelease/);
  assert.match(publish, /--latest=false/);
  assert.match(publish, /--draft/);
  assert.doesNotMatch(publish, /latest\.json/);
  assert.doesNotMatch(publish, /\.app\.tar\.gz/);
  assert.match(stable, /workflow_dispatch: \{\}/);
  assert.doesNotMatch(stable, /cron:/);
  assert.doesNotMatch(stable, /--prerelease/);
});

test("nightly publish is a prerelease no-op when the tag already exists", (t) => {
  const { dir, env } = fixture(t);
  const workflow = readFileSync(join(root, ".github/workflows/nightly-macos.yml"), "utf8");
  const script = runScript(workflow, "Publish nightly prerelease");
  mkdirSync(join(dir, "src-tauri/target/release/bundle/dmg"), { recursive: true });
  writeFileSync(join(dir, "src-tauri/tauri.conf.json"), '{"version":"0.5.10"}');
  const dmg = join(dir, "src-tauri/target/release/bundle/dmg/Bridge_0.5.10_x64.dmg");
  writeFileSync(dmg, "dmg");
  writeFileSync(`${dmg}.sha256`, "checksum");
  writeFileSync(join(dir, "notes.md"), "notes");
  executable(join(dir, "bin/uname"), "#!/bin/sh\nprintf '%s\\n' x86_64\n");
  const createdFlag = join(dir, "created");
  executable(join(dir, "bin/gh"), `#!/usr/bin/env node
const fs = require("fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.GH_LOG, JSON.stringify(args) + "\\n");
const joined = args.join(" ");
if (joined.includes("release view") || joined.includes("/git/ref/tags/")) {
  const raced = process.env.EXISTS_AFTER_CREATE === "1" && fs.existsSync(process.env.CREATED_FLAG);
  if (process.env.EXISTS === "1" || raced) process.exit(0);
  process.exit(1);
}
if (args[1] === "create") {
  fs.writeFileSync(process.env.CREATED_FLAG, "1");
  if (process.env.FAIL_CREATE === "1") process.exit(1);
}
`);
  const run = (extra) => {
    const log = join(dir, "gh.log");
    writeFileSync(log, "");
    rmSync(createdFlag, { force: true });
    const out = spawnSync("/bin/bash", ["--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script], {
      cwd: dir,
      env: {
        ...env,
        GH_LOG: log,
        CREATED_FLAG: createdFlag,
        GITHUB_REPOSITORY: "Atharva-Kanherkar/bridge-harness",
        NIGHTLY_TAG: "nightly-2026-09-23",
        NIGHTLY_DATE: "2026-09-23",
        NIGHTLY_SHA: sha,
        NIGHTLY_NOTES_FILE: join(dir, "notes.md"),
        ...extra,
      },
      encoding: "utf8",
    });
    const calls = readFileSync(log, "utf8").trim().split("\n").filter(Boolean).map((line) => JSON.parse(line));
    return { out, calls };
  };
  const exists = run({ EXISTS: "1" });
  assert.equal(exists.out.status, 0, exists.out.stderr);
  assert.equal(exists.calls.some((args) => args.includes("create")), false);
  const created = run({});
  assert.equal(created.out.status, 0, created.out.stderr);
  assert.ok(created.calls[2].includes("--prerelease"));
  assert.ok(created.calls[2].includes("--latest=false"));
  assert.ok(created.calls[2].includes("--draft"));
  assert.ok(created.calls[2].includes("--target"));
  assert.equal(created.calls[2].includes("latest.json"), false);
  assert.deepEqual(created.calls[3].slice(0, 3), ["release", "edit", "nightly-2026-09-23"]);
  assert.ok(created.calls[3].includes("--draft=false"));
  assert.ok(created.calls[3].includes("--prerelease"));
  const failed = run({ FAIL_CREATE: "1" });
  assert.notEqual(failed.out.status, 0);
  assert.equal(failed.calls.some((args) => args[1] === "edit"), false);
  const raced = run({ FAIL_CREATE: "1", EXISTS_AFTER_CREATE: "1" });
  assert.equal(raced.out.status, 0, raced.out.stderr);
  assert.equal(raced.calls.some((args) => args[1] === "edit"), false);
  const badTag = run({ NIGHTLY_TAG: "v0.5.10", NIGHTLY_DATE: "v0.5.10" });
  assert.notEqual(badTag.out.status, 0);
  assert.equal(badTag.calls.length, 0);
});
