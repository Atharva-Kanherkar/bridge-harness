import { test } from "node:test";
import assert from "node:assert/strict";
import {
  chmodSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import {
  cargoLockPackageVersions,
  syncGeneratedLockVersions,
  updateCargoLockVersions,
  verifyReleaseVersions,
} from "../release-version.mjs";
import {
  checkPullRequestTitle,
  isConventionalPullRequestTitle,
} from "../check-pull-request-title.mjs";
import { createUpdaterManifest } from "../create-updater-manifest.mjs";
import { verifyUpdaterSignatureBytes } from "../verify-updater-signature.mjs";
import {
  completePublishedAssetSet,
  releasePullRequestFor,
  releasePullRequestTitle,
  releaseTagForVersion,
  resolveProductionRelease,
} from "../resolve-production-release.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const updaterTestPublicBox = [
  "untrusted comment: minisign public key E7620F1842B4E81F",
  "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
].join("\n");
const updaterTestSignatureBox = [
  "untrusted comment: signature from minisign secret key",
  "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=",
  "trusted comment: timestamp:1556193335\tfile:test",
  "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==",
].join("\n");
const updaterTestPublicKey = Buffer.from(updaterTestPublicBox).toString("base64");
const updaterTestSignature = Buffer.from(updaterTestSignatureBox).toString("base64");

function fixture(t, version = "0.5.8") {
  const dir = mkdtempSync(join(tmpdir(), "bridge-release-pipeline-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, "browser-extension"), { recursive: true });
  mkdirSync(join(dir, "safari-extension/Resources"), { recursive: true });
  mkdirSync(join(dir, "sidecar/claude-agent"), { recursive: true });
  mkdirSync(join(dir, "src-tauri/bridge-core"), { recursive: true });
  mkdirSync(join(dir, "src-tauri/bridge-menu-bar"), { recursive: true });
  mkdirSync(join(dir, "packaging/arch"), { recursive: true });
  writeFileSync(join(dir, "package.json"), JSON.stringify({ name: "bridge-deck", version }));
  writeFileSync(join(dir, ".release-please-manifest.json"), JSON.stringify({ ".": version }));
  writeFileSync(
    join(dir, "browser-extension/manifest.json"),
    JSON.stringify({ manifest_version: 3, name: "Bridge Authenticated Browser", version }),
  );
  writeFileSync(
    join(dir, "safari-extension/Resources/manifest.json"),
    JSON.stringify({ manifest_version: 3, name: "Bridge Authenticated Browser for Safari", version }),
  );
  writeFileSync(
    join(dir, "sidecar/claude-agent/package.json"),
    JSON.stringify({ name: "bridge-claude-agent-sidecar", version }),
  );
  writeFileSync(
    join(dir, "sidecar/claude-agent/package-lock.json"),
    `${JSON.stringify(
      {
        name: "bridge-claude-agent-sidecar",
        version,
        lockfileVersion: 3,
        requires: true,
        packages: {
          "": {
            name: "bridge-claude-agent-sidecar",
            version,
            dependencies: { fixture: "9.9.9" },
          },
          "node_modules/fixture": { version: "9.9.9" },
        },
      },
      null,
      2,
    )}\n`,
  );
  writeFileSync(
    join(dir, "bun.lock"),
    `{
  "lockfileVersion": 1,
  "workspaces": {
    "sidecar/claude-agent": {
      "name": "bridge-claude-agent-sidecar",
      "version": "${version}",
      "dependencies": {
        "fixture": "9.9.9",
      },
    },
  },
  "packages": {
    "fixture": ["fixture@9.9.9"],
  },
}
`,
  );
  writeFileSync(join(dir, "src-tauri/tauri.conf.json"), JSON.stringify({ version }));
  writeFileSync(
    join(dir, "src-tauri/Cargo.toml"),
    `[workspace]\nmembers = ["bridge-core", "bridge-menu-bar"]\n\n[workspace.package]\nversion = "${version}"\n\n[package]\nname = "bridge-deck"\nversion = { workspace = true }\n`,
  );
  writeFileSync(
    join(dir, "src-tauri/bridge-core/Cargo.toml"),
    '[package]\nname = "bridge-core"\nversion = { workspace = true }\n',
  );
  writeFileSync(
    join(dir, "src-tauri/bridge-menu-bar/Cargo.toml"),
    '[package]\nname = "bridge-menu-bar"\nversion.workspace = true\n',
  );
  writeFileSync(
    join(dir, "src-tauri/Cargo.lock"),
    `version = 4\n\n[[package]]\nname = "bridge-core"\nversion = "${version}"\n\n[[package]]\nname = "bridge-deck"\nversion = "${version}"\n\n[[package]]\nname = "bridge-menu-bar"\nversion = "${version}"\n\n[[package]]\nname = "external"\nversion = "9.9.9"\nsource = "registry+fixture"\n`,
  );
  writeFileSync(
    join(dir, "packaging/arch/PKGBUILD"),
    `pkgver=${version} # x-release-please-version\n`,
  );
  return dir;
}

test("Release Please has one root app release and every authoritative version target", () => {
  const config = JSON.parse(readFileSync(join(root, "release-please-config.json")));
  const manifest = JSON.parse(readFileSync(join(root, ".release-please-manifest.json")));
  const packageMetadata = JSON.parse(readFileSync(join(root, "package.json")));
  assert.deepEqual(Object.keys(config.packages), ["."]);
  assert.deepEqual(manifest, { ".": packageMetadata.version });
  assert.equal(config["include-component-in-tag"], false);
  assert.equal(config["include-v-in-tag"], true);
  assert.equal(config.draft, true);
  assert.equal(config["force-tag-creation"], true);
  assert.equal(
    config["separate-pull-requests"],
    true,
    "the single package must retain its component-bearing release branch",
  );
  assert.equal(config["group-pull-request-title-pattern"], "chore${scope}: release ${version}");
  assert.equal(
    releasePullRequestTitle(config, "0.5.9"),
    "chore(main): release 0.5.9",
    "the pinned Release Please config deliberately omits the root component and v prefix from its PR title",
  );
  assert.equal(config["bump-minor-pre-major"], undefined, "breaking changes must bump major");
  assert.equal(config["bump-patch-for-minor-pre-major"], undefined, "features must bump minor");
  const release = config.packages["."];
  assert.equal(release["release-type"], undefined, "the root node strategy should be inherited");
  assert.equal(release["pull-request-title-pattern"], "chore${scope}: release ${version}");
  assert.deepEqual(
    release["extra-files"].map((entry) => entry.path).sort(),
    [
      "browser-extension/manifest.json",
      "packaging/arch/PKGBUILD",
      "safari-extension/Resources/manifest.json",
      "sidecar/claude-agent/package.json",
      "src-tauri/Cargo.toml",
      "src-tauri/tauri.conf.json",
    ],
  );
  assert.ok(release["exclude-paths"].includes("landing"));
  verifyReleaseVersions(root);
});

test("generated lock synchronization updates app versions and no dependency", (t) => {
  const dir = fixture(t);
  const cargoLockPath = join(dir, "src-tauri/Cargo.lock");
  const packageLockPath = join(dir, "sidecar/claude-agent/package-lock.json");
  const bunLockPath = join(dir, "bun.lock");
  writeFileSync(
    cargoLockPath,
    readFileSync(cargoLockPath, "utf8").replace(
      'name = "bridge-core"\nversion = "0.5.8"',
      'name = "bridge-core"\nversion = "0.5.7"',
    ),
  );
  const packageLock = JSON.parse(readFileSync(packageLockPath, "utf8"));
  packageLock.version = "0.5.6";
  packageLock.packages[""].version = "0.5.7";
  writeFileSync(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`);
  writeFileSync(
    bunLockPath,
    readFileSync(bunLockPath, "utf8").replace('"version": "0.5.8"', '"version": "0.5.5"'),
  );
  assert.throws(() => verifyReleaseVersions(dir), /bridge-core version 0\.5\.7/);
  assert.equal(syncGeneratedLockVersions(dir), true);
  const cargoAfter = readFileSync(cargoLockPath, "utf8");
  assert.match(cargoAfter, /name = "external"\nversion = "9\.9\.9"/);
  const packageAfter = JSON.parse(readFileSync(packageLockPath, "utf8"));
  assert.equal(packageAfter.version, "0.5.8");
  assert.equal(packageAfter.packages[""].version, "0.5.8");
  assert.equal(packageAfter.packages["node_modules/fixture"].version, "9.9.9");
  const bunAfter = readFileSync(bunLockPath, "utf8");
  assert.match(
    bunAfter,
    /"sidecar\/claude-agent": \{[\s\S]*?"version": "0\.5\.8"/,
  );
  assert.match(bunAfter, /"fixture": \["fixture@9\.9\.9"\]/);
  assert.equal(syncGeneratedLockVersions(dir), false);
  verifyReleaseVersions(dir);

  writeFileSync(join(dir, "packaging/arch/PKGBUILD"), "pkgver=0.5.8\n");
  assert.throws(() => verifyReleaseVersions(dir), /PKGBUILD has no annotated pkgver/);
});

test("release version CLI can synchronize a separate checkout from trusted tooling", (t) => {
  const dir = fixture(t);
  const cargoLockPath = join(dir, "src-tauri/Cargo.lock");
  writeFileSync(
    cargoLockPath,
    readFileSync(cargoLockPath, "utf8").replace(
      'name = "bridge-deck"\nversion = "0.5.8"',
      'name = "bridge-deck"\nversion = "0.5.7"',
    ),
  );
  const result = spawnSync(
    process.execPath,
    [join(root, "scripts/release-version.mjs"), "--root", dir, "--write-locks"],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Synchronized generated lockfile versions/);
  verifyReleaseVersions(dir);
});

test("release validation covers extension, sidecar, and generated lock versions", (t) => {
  const cases = [
    ["browser-extension/manifest.json", /browser-extension\/manifest\.json version 0\.5\.7/],
    [
      "safari-extension/Resources/manifest.json",
      /safari-extension\/Resources\/manifest\.json version 0\.5\.7/,
    ],
    [
      "sidecar/claude-agent/package.json",
      /sidecar\/claude-agent\/package\.json version 0\.5\.7/,
    ],
  ];
  for (const [path, expected] of cases) {
    const dir = fixture(t);
    const metadata = JSON.parse(readFileSync(join(dir, path), "utf8"));
    metadata.version = "0.5.7";
    writeFileSync(join(dir, path), JSON.stringify(metadata));
    assert.throws(() => verifyReleaseVersions(dir), expected);
  }

  {
    const dir = fixture(t);
    const path = join(dir, "sidecar/claude-agent/package-lock.json");
    const lock = JSON.parse(readFileSync(path, "utf8"));
    lock.version = "0.5.7";
    writeFileSync(path, JSON.stringify(lock));
    assert.throws(() => verifyReleaseVersions(dir), /package-lock\.json version 0\.5\.7/);
  }
  {
    const dir = fixture(t);
    const path = join(dir, "sidecar/claude-agent/package-lock.json");
    const lock = JSON.parse(readFileSync(path, "utf8"));
    lock.packages[""].version = "0.5.7";
    writeFileSync(path, JSON.stringify(lock));
    assert.throws(
      () => verifyReleaseVersions(dir),
      /package-lock\.json packages\[""\] version 0\.5\.7/,
    );
  }
  {
    const dir = fixture(t);
    const path = join(dir, "bun.lock");
    writeFileSync(
      path,
      readFileSync(path, "utf8").replace('"version": "0.5.8"', '"version": "0.5.7"'),
    );
    assert.throws(
      () => verifyReleaseVersions(dir),
      /bun\.lock workspaces\["sidecar\/claude-agent"\] version 0\.5\.7/,
    );
  }
});

test("Cargo lock helpers reject missing and duplicate workspace packages", () => {
  const lock = '[[package]]\nname = "bridge-core"\nversion = "0.5.8"\n';
  assert.throws(
    () => cargoLockPackageVersions(lock, ["bridge-core", "bridged"]),
    /missing workspace package bridged/,
  );
  assert.throws(
    () => cargoLockPackageVersions(lock + lock, ["bridge-core"]),
    /duplicate workspace package bridge-core/,
  );
  assert.throws(
    () => updateCargoLockVersions(lock, ["bridge-core", "bridged"], "0.5.9"),
    /missing workspace package bridged/,
  );
});

test("PR title policy gives Release Please one deliberate SemVer input", () => {
  for (const title of [
    "fix: handle daemon startup race",
    "feat(settings): add provider configuration",
    "perf(daemon): reduce handshake latency",
    "feat!: replace the session format",
    "feat(protocol)!: replace the protocol",
    "fix: x",
    "chore(main): release 0.6.0",
  ]) {
    assert.equal(isConventionalPullRequestTitle(title), true, title);
    assert.doesNotThrow(() => checkPullRequestTitle(title));
  }
  for (const title of ["Add stable platform downloads", "fix daemon race", "feat(): empty scope", "feat: "]) {
    assert.equal(isConventionalPullRequestTitle(title), false, title);
    assert.throws(() => checkPullRequestTitle(title), /Conventional Commit syntax/);
  }
});

test("updater metadata is immutable to the release tag and architecture", () => {
  const input = {
    version: "0.5.9",
    tag: "v0.5.9",
    arch: "aarch64",
    signature: " signed-fixture \n",
    repository: "Atharva-Kanherkar/bridge-harness",
    updaterName: "Bridge_0.5.9_aarch64.app.tar.gz",
    notes: "Fixed startup.",
    pubDate: "2026-09-13T12:00:00Z",
  };
  const manifest = createUpdaterManifest(input);
  assert.equal(manifest.version, "0.5.9");
  assert.deepEqual(Object.keys(manifest.platforms), ["darwin-aarch64"]);
  assert.equal(manifest.platforms["darwin-aarch64"].signature, "signed-fixture");
  assert.equal(
    manifest.platforms["darwin-aarch64"].url,
    "https://github.com/Atharva-Kanherkar/bridge-harness/releases/download/v0.5.9/Bridge_0.5.9_aarch64.app.tar.gz",
  );
  assert.throws(
    () => createUpdaterManifest({ ...input, tag: "v0.6.0" }),
    /does not match/,
  );
  assert.throws(
    () => createUpdaterManifest({ ...input, updaterName: "Bridge_0.6.0_aarch64.app.tar.gz" }),
    /Invalid updater artifact name/,
  );
  const intelManifest = createUpdaterManifest({
    ...input,
    arch: "x64",
    updaterName: "Bridge_0.5.9_x64.app.tar.gz",
  });
  assert.deepEqual(Object.keys(intelManifest.platforms), ["darwin-x86_64"]);
});

test("updater artifacts must verify against the configured key", () => {
  assert.doesNotThrow(() =>
    verifyUpdaterSignatureBytes(Buffer.from("test"), updaterTestSignature, updaterTestPublicKey),
  );
  assert.throws(
    () =>
      verifyUpdaterSignatureBytes(
        Buffer.from("tampered"),
        updaterTestSignature,
        updaterTestPublicKey,
      ),
    /did not verify/,
  );

  const publicLines = updaterTestPublicBox.split("\n");
  const wrongPayload = Buffer.from(publicLines[1], "base64");
  wrongPayload[wrongPayload.length - 1] ^= 1;
  const wrongKey = Buffer.from(`${publicLines[0]}\n${wrongPayload.toString("base64")}`).toString(
    "base64",
  );
  assert.throws(
    () => verifyUpdaterSignatureBytes(Buffer.from("test"), updaterTestSignature, wrongKey),
    /did not verify/,
  );
});

function initDmgSmokeFixture(t) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-dmg-smoke-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const scripts = join(dir, "scripts");
  const bin = join(dir, "bin");
  const artifacts = join(dir, "artifacts");
  mkdirSync(scripts);
  mkdirSync(bin);
  mkdirSync(artifacts);
  cpSync(join(root, "scripts/smoke-packaged-macos-dmg.sh"), join(scripts, "smoke-packaged-macos-dmg.sh"));
  writeFileSync(join(scripts, "smoke-packaged-macos-app.py"), "# fixture path\n");

  const dmg = join(artifacts, "Bridge_0.5.2_aarch64.dmg");
  writeFileSync(dmg, "fixture dmg\n");
  const digest = createHash("sha256").update(readFileSync(dmg)).digest("hex");
  writeFileSync(`${dmg}.sha256`, `${digest}  ${dmg.split("/").pop()}\n`);
  const events = join(dir, "events");
  writeFileSync(events, "");
  const tool = (name, source) => {
    const path = join(bin, name);
    writeFileSync(path, source);
    chmodSync(path, 0o755);
  };
  tool(
    "hdiutil",
    `#!/usr/bin/env node
const fs = require("fs"), path = require("path"), args = process.argv.slice(2);
fs.appendFileSync(process.env.EVENTS, JSON.stringify(["hdiutil", ...args]) + "\\n");
if (args[0] === "attach") {
  const mount = args[args.indexOf("-mountpoint") + 1];
  fs.mkdirSync(path.join(mount, "Bridge.app"));
} else if (args[0] === "detach") {
  fs.rmSync(path.join(args[1], "Bridge.app"), { recursive: true });
}
`,
  );
  tool(
    "python3",
    `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
fs.appendFileSync(process.env.EVENTS, JSON.stringify(["python3", ...args]) + "\\n");
if (process.env.FAIL_SMOKE) process.exit(1);
`,
  );
  return { dir, bin, dmg, events };
}

test("release DMG smoke verifies the checksum, mounts read-only, and always detaches", (t) => {
  const { dir, bin, dmg, events } = initDmgSmokeFixture(t);
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    EVENTS: events,
    TMPDIR: dir,
  };
  const run = (extra = {}) =>
    spawnSync("/bin/sh", [join(dir, "scripts/smoke-packaged-macos-dmg.sh"), dmg], {
      cwd: dir,
      env: { ...env, ...extra },
      encoding: "utf8",
    });

  let out = run();
  assert.equal(out.status, 0, out.stderr);
  let calls = readFileSync(events, "utf8").trim().split("\n").map((line) => JSON.parse(line));
  assert.deepEqual(calls.map((call) => call[0]), ["hdiutil", "python3", "hdiutil"]);
  assert.ok(calls[0].includes("-readonly"));
  assert.ok(calls[0].includes("-nobrowse"));
  assert.match(calls[1].at(-1), /\/Bridge\.app$/);

  writeFileSync(events, "");
  out = run({ FAIL_SMOKE: "1" });
  assert.notEqual(out.status, 0);
  calls = readFileSync(events, "utf8").trim().split("\n").map((line) => JSON.parse(line));
  assert.deepEqual(calls.map((call) => call[0]), ["hdiutil", "python3", "hdiutil"]);

  writeFileSync(events, "");
  writeFileSync(dmg, "tampered dmg\n");
  out = run();
  assert.notEqual(out.status, 0);
  assert.equal(readFileSync(events, "utf8"), "", "a bad checksum must fail before mounting");
});

test("release authorization requires the generated PR title and tagged label", () => {
  const config = JSON.parse(readFileSync(join(root, "release-please-config.json")));
  const mergeCommitSha = "0123456789abcdef0123456789abcdef01234567";
  const valid = {
    base: { ref: "main" },
    merged_at: "2026-09-13T12:00:00Z",
    merge_commit_sha: mergeCommitSha,
    title: "chore(main): release 0.5.9",
    labels: [{ name: "autorelease: tagged" }],
  };
  assert.equal(releasePullRequestFor([valid], "0.5.9", config, mergeCommitSha), valid);
  assert.equal(
    releasePullRequestFor([{ ...valid, labels: [] }], "0.5.9", config, mergeCommitSha),
    undefined,
  );
  assert.equal(
    releasePullRequestFor(
      [{ ...valid, title: "chore: bump version" }],
      "0.5.9",
      config,
      mergeCommitSha,
    ),
    undefined,
  );
  assert.equal(
    releasePullRequestFor(
      [{ ...valid, merge_commit_sha: "fedcba9876543210fedcba9876543210fedcba98" }],
      "0.5.9",
      config,
      mergeCommitSha,
    ),
    undefined,
  );
  assert.equal(releaseTagForVersion("0.5.9"), "v0.5.9");
  const assets = [
    "Bridge_0.5.9_aarch64.dmg",
    "Bridge_0.5.9_aarch64.dmg.sha256",
    "Bridge_0.5.9_aarch64.app.tar.gz",
    "Bridge_0.5.9_aarch64.app.tar.gz.sig",
    "latest.json",
  ].map((name) => ({ name }));
  assert.equal(completePublishedAssetSet(assets, "0.5.9"), true);
  assert.equal(completePublishedAssetSet(assets.slice(1), "0.5.9"), false);
  assert.equal(completePublishedAssetSet([...assets, { name: "unexpected.txt" }], "0.5.9"), false);
});

test("release recovery resolves an ancestor tag instead of a later successful main head", (t) => {
  const dir = fixture(t);
  writeFileSync(
    join(dir, "release-please-config.json"),
    JSON.stringify({
      "include-component-in-tag": false,
      "group-pull-request-title-pattern": "chore${scope}: release ${version}",
      packages: { ".": {} },
    }),
  );
  const gitEnv = {
    ...process.env,
    GIT_AUTHOR_NAME: "Fixture",
    GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "Fixture",
    GIT_COMMITTER_EMAIL: "fixture@example.invalid",
  };
  const git = (args) => {
    const out = spawnSync("git", args, { cwd: dir, env: gitEnv, encoding: "utf8" });
    assert.equal(out.status, 0, out.stderr);
    return out.stdout.trim();
  };
  git(["init", "-q"]);
  git(["add", "."]);
  git(["commit", "-qm", "chore(main): release 0.5.8"]);
  const releaseSha = git(["rev-parse", "HEAD"]);
  git(["tag", "v0.5.8"]);
  writeFileSync(join(dir, "later-main-change"), "later successful main head\n");
  git(["add", "later-main-change"]);
  git(["commit", "-qm", "chore: later main change"]);
  const successfulMainSha = git(["rev-parse", "HEAD"]);

  const bin = join(dir, "bin");
  mkdirSync(bin);
  const calls = join(dir, "gh-calls");
  const gh = join(bin, "gh");
  writeFileSync(
    gh,
    `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
fs.appendFileSync(process.env.GH_CALLS, JSON.stringify(args) + "\\n");
if (args[0] === "release" && args[1] === "view") {
  console.log(JSON.stringify({isDraft:true,isPrerelease:false,tagName:"v0.5.8",assets:[]}));
} else if (args[0] === "api") {
  console.log(JSON.stringify([{
    base:{ref:"main"},
    merged_at:"2026-09-13T12:00:00Z",
    merge_commit_sha:process.env.RELEASE_MERGE_SHA,
    title:"chore(main): release 0.5.8",
    labels:[{name:"autorelease: tagged"}],
  }]));
} else {
  process.exit(1);
}
`,
  );
  chmodSync(gh, 0o755);
  const output = join(dir, "github-output");
  const released = resolveProductionRelease({
    sourceSha: successfulMainSha,
    repository: "example/bridge",
    checkoutRoot: dir,
    commandEnv: {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      GH_CALLS: calls,
      RELEASE_MERGE_SHA: releaseSha,
    },
    output,
  });

  assert.equal(released, true);
  const outputs = Object.fromEntries(
    readFileSync(output, "utf8")
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  );
  assert.deepEqual(outputs, {
    should_release: "true",
    release_tag: "v0.5.8",
    release_version: "0.5.8",
    release_sha: releaseSha,
  });
  const ghCalls = readFileSync(calls, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.ok(
    ghCalls.some(
      (args) => args[0] === "api" && args.at(-1) === `repos/example/bridge/commits/${releaseSha}/pulls`,
    ),
  );
});

test("a complete public legacy release no-ops before Release Please provenance lookup", (t) => {
  const dir = fixture(t);
  const gitEnv = {
    ...process.env,
    GIT_AUTHOR_NAME: "Fixture",
    GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "Fixture",
    GIT_COMMITTER_EMAIL: "fixture@example.invalid",
  };
  const git = (args) => {
    const out = spawnSync("git", args, { cwd: dir, env: gitEnv, encoding: "utf8" });
    assert.equal(out.status, 0, out.stderr);
    return out.stdout.trim();
  };
  git(["init", "-q"]);
  git(["add", "."]);
  git(["commit", "-qm", "legacy public release"]);
  const releaseSha = git(["rev-parse", "HEAD"]);
  git(["tag", "v0.5.8"]);

  // The tagged release predates Release Please. Only the later successful main
  // head contains its configuration, so a public no-op must not inspect PR
  // provenance from the legacy tag.
  writeFileSync(
    join(dir, "release-please-config.json"),
    JSON.stringify({ "bootstrap-sha": releaseSha, packages: { ".": {} } }),
  );
  git(["add", "release-please-config.json"]);
  git(["commit", "-qm", "ci: add Release Please"]);
  const successfulMainSha = git(["rev-parse", "HEAD"]);

  const bin = join(dir, "bin");
  mkdirSync(bin);
  const calls = join(dir, "gh-calls");
  const gh = join(bin, "gh");
  writeFileSync(
    gh,
    `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
fs.appendFileSync(process.env.GH_CALLS, JSON.stringify(args) + "\\n");
if (args[0] === "release" && args[1] === "view") {
  const assets = [
    "Bridge_0.5.8_aarch64.dmg",
    "Bridge_0.5.8_aarch64.dmg.sha256",
    "Bridge_0.5.8_aarch64.app.tar.gz",
    "Bridge_0.5.8_aarch64.app.tar.gz.sig",
    "latest.json",
  ].map((name) => ({ name }));
  console.log(JSON.stringify({
    isDraft:false,
    isPrerelease:false,
    tagName:"v0.5.8",
    assets:process.env.INCOMPLETE_RELEASE ? assets.slice(1) : assets,
  }));
} else {
  console.error("legacy release must not query commit-to-PR provenance");
  process.exit(42);
}
`,
  );
  chmodSync(gh, 0o755);
  const commandEnv = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    GH_CALLS: calls,
  };
  const output = join(dir, "github-output");
  const released = resolveProductionRelease({
    sourceSha: successfulMainSha,
    repository: "example/bridge",
    checkoutRoot: dir,
    commandEnv,
    output,
  });

  assert.equal(released, false);
  const outputs = Object.fromEntries(
    readFileSync(output, "utf8")
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  );
  assert.deepEqual(outputs, {
    should_release: "false",
    release_tag: "v0.5.8",
    release_version: "0.5.8",
    release_sha: releaseSha,
  });

  assert.throws(
    () =>
      resolveProductionRelease({
        sourceSha: successfulMainSha,
        repository: "example/bridge",
        checkoutRoot: dir,
        commandEnv: { ...commandEnv, INCOMPLETE_RELEASE: "1" },
        output: join(dir, "incomplete-output"),
      }),
    /missing required macOS or updater assets/,
  );
  const ghCalls = readFileSync(calls, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  assert.equal(ghCalls.length, 2);
  assert.ok(ghCalls.every((args) => args[0] === "release" && args[1] === "view"));
});

function initPublishFixture(t) {
  const dir = fixture(t, "0.5.2");
  writeFileSync(
    join(dir, "src-tauri/tauri.conf.json"),
    JSON.stringify({ version: "0.5.2", plugins: { updater: { pubkey: updaterTestPublicKey } } }),
  );
  mkdirSync(join(dir, "scripts"));
  for (const name of [
    "release-version.mjs",
    "create-updater-manifest.mjs",
    "verify-updater-signature.mjs",
    "publish-github-release.sh",
  ]) {
    cpSync(join(root, "scripts", name), join(dir, "scripts", name));
  }
  const gitEnv = {
    ...process.env,
    GIT_AUTHOR_NAME: "Fixture",
    GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "Fixture",
    GIT_COMMITTER_EMAIL: "fixture@example.invalid",
  };
  for (const args of [["init", "-q"], ["add", "."], ["commit", "-qm", "fixture"], ["tag", "v0.5.2"]]) {
    const out = spawnSync("git", args, { cwd: dir, env: gitEnv, encoding: "utf8" });
    assert.equal(out.status, 0, out.stderr);
  }
  const sha = spawnSync("git", ["rev-parse", "HEAD"], { cwd: dir, encoding: "utf8" }).stdout.trim();
  const artifacts = join(dir, "artifacts");
  mkdirSync(artifacts);
  const names = [
    "Bridge_0.5.2_aarch64.dmg",
    "Bridge_0.5.2_aarch64.app.tar.gz",
    "Bridge_0.5.2_aarch64.app.tar.gz.sig",
  ];
  for (const name of names) writeFileSync(join(artifacts, name), `fixture ${name}\n`);
  writeFileSync(join(artifacts, "Bridge_0.5.2_aarch64.app.tar.gz"), "test");
  writeFileSync(join(artifacts, "Bridge_0.5.2_aarch64.app.tar.gz.sig"), updaterTestSignature);
  const dmgName = names[0];
  const digest = createHash("sha256").update(readFileSync(join(artifacts, dmgName))).digest("hex");
  writeFileSync(join(artifacts, `${dmgName}.sha256`), `${digest}  ${dmgName}\n`);

  const bin = join(dir, "bin");
  mkdirSync(bin);
  const events = join(dir, "events");
  const gh = join(bin, "gh");
  writeFileSync(
    gh,
    `#!/usr/bin/env node
const fs = require("fs"), args = process.argv.slice(2);
fs.appendFileSync(process.env.EVENTS, JSON.stringify(args) + "\\n");
if (args[0] === "release" && args[1] === "upload") {
  if (process.env.FAIL_UPLOAD) process.exit(1);
} else if (args.includes("isDraft")) {
  console.log(process.env.PUBLIC_RELEASE ? "false" : "true");
} else if (args.includes("body")) {
  console.log("Fixture release notes");
} else if (args.includes("assets")) {
  const assets = ["Bridge_0.5.2_aarch64.dmg","Bridge_0.5.2_aarch64.dmg.sha256","Bridge_0.5.2_aarch64.app.tar.gz","Bridge_0.5.2_aarch64.app.tar.gz.sig","latest.json"];
  console.log((process.env.INCOMPLETE_ASSETS ? assets.slice(1) : assets).join("\\n"));
}
`,
  );
  chmodSync(gh, 0o755);
  return { dir, artifacts, events, sha, bin };
}

test("publication leaves the existing release draft until every asset is visible", (t) => {
  const { dir, artifacts, events, sha, bin } = initPublishFixture(t);
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    EVENTS: events,
    GH_TOKEN: "fixture",
    GITHUB_REPOSITORY: "example/bridge",
    RELEASE_TAG: "v0.5.2",
    RELEASE_VERSION: "0.5.2",
    RELEASE_SHA: sha,
    RELEASE_ARTIFACT_DIR: artifacts,
    TMPDIR: dir,
  };
  for (const fail of [true, false]) {
    writeFileSync(events, "");
    const out = spawnSync("/bin/sh", [join(dir, "scripts/publish-github-release.sh")], {
      cwd: dir,
      env: { ...env, FAIL_UPLOAD: fail ? "1" : "" },
      encoding: "utf8",
    });
    assert.equal(out.status === 0, !fail, out.stderr);
    const calls = readFileSync(events, "utf8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line));
    const upload = calls.find((args) => args[0] === "release" && args[1] === "upload");
    assert.ok(upload?.includes("--clobber"));
    const edits = calls.filter((args) => args[0] === "release" && args[1] === "edit");
    if (fail) assert.equal(edits.length, 0, "an upload failure must leave the draft unpublished");
    else assert.deepEqual(edits, [["release", "edit", "v0.5.2", "--repo", "example/bridge", "--draft=false", "--latest"]]);
  }
  const latest = JSON.parse(readFileSync(join(artifacts, "latest.json")));
  assert.match(latest.platforms["darwin-aarch64"].url, /releases\/download\/v0\.5\.2\//);

  writeFileSync(events, "");
  const alreadyPublic = spawnSync("/bin/sh", [join(dir, "scripts/publish-github-release.sh")], {
    cwd: dir,
    env: { ...env, PUBLIC_RELEASE: "1" },
    encoding: "utf8",
  });
  assert.equal(alreadyPublic.status, 0, alreadyPublic.stderr);
  let calls = readFileSync(events, "utf8")
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  assert.equal(calls.some((args) => args[0] === "release" && args[1] === "upload"), false);
  assert.equal(calls.some((args) => args[0] === "release" && args[1] === "edit"), false);

  writeFileSync(events, "");
  const incompletePublic = spawnSync("/bin/sh", [join(dir, "scripts/publish-github-release.sh")], {
    cwd: dir,
    env: { ...env, PUBLIC_RELEASE: "1", INCOMPLETE_ASSETS: "1" },
    encoding: "utf8",
  });
  assert.notEqual(incompletePublic.status, 0);
  assert.match(incompletePublic.stderr, /expected exactly five release assets/);
  calls = readFileSync(events, "utf8")
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  assert.equal(calls.some((args) => args[0] === "release" && args[1] === "upload"), false);
  assert.equal(calls.some((args) => args[0] === "release" && args[1] === "edit"), false);

  const newerTag = spawnSync("git", ["tag", "v0.5.3"], { cwd: dir, encoding: "utf8" });
  assert.equal(newerTag.status, 0, newerTag.stderr);
  writeFileSync(events, "");
  const superseded = spawnSync("/bin/sh", [join(dir, "scripts/publish-github-release.sh")], {
    cwd: dir,
    env,
    encoding: "utf8",
  });
  assert.notEqual(superseded.status, 0);
  assert.match(superseded.stderr, /superseded by newer version tag v0\.5\.3/);
  assert.equal(readFileSync(events, "utf8"), "", "a superseded release must not call GitHub");
});

test("workflow boundaries keep PR smoke and post-build acceptance credential-free", () => {
  const ci = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
  const pullRequestPolicy = readFileSync(join(root, ".github/workflows/pr-title.yml"), "utf8");
  const linuxTauriConfig = JSON.parse(
    readFileSync(join(root, "src-tauri/tauri.linux.conf.json"), "utf8"),
  );
  const releasePlease = readFileSync(join(root, ".github/workflows/release-please.yml"), "utf8");
  const releasePleaseJob = releasePlease
    .split("  release-please:\n")[1]
    .split("\n  production-macos:\n")[0];
  const productionCall = releasePlease.split("  production-macos:\n")[1];
  const appToken = releasePleaseJob
    .split("      - name: Create repository-scoped release token\n")[1]
    .split("\n      - name: Require protected pull request inputs\n")[0];
  const protectedInputs = releasePleaseJob
    .split("      - name: Require protected pull request inputs\n")[1]
    .split("\n      - name: Require deterministic squash-only merges\n")[0];
  const mergePolicy = releasePleaseJob
    .split("      - name: Require deterministic squash-only merges\n")[1]
    .split("\n      - name: Create or update the cumulative Release PR\n")[0];
  const releasePrCheckout = releasePleaseJob
    .split("      - name: Check out the Release PR as untrusted data\n")[1]
    .split("\n      - name: Check out release tooling from protected main\n")[0];
  const trustedToolsCheckout = releasePleaseJob
    .split("      - name: Check out release tooling from protected main\n")[1]
    .split("\n      # actions/setup-node")[0];
  const lockSync = releasePleaseJob
    .split("      - name: Synchronize generated lockfile versions\n")[1]
    .split("\n      - name: Commit lockfiles when Release Please changed the version\n")[0];
  const lockCommit = releasePleaseJob
    .split("      - name: Commit lockfiles when Release Please changed the version\n")[1];
  const production = readFileSync(join(root, ".github/workflows/release-macos.yml"), "utf8");
  const releaseDmg = readFileSync(join(root, "scripts/release-dmg.sh"), "utf8");
  const preflight = production.split("  preflight:\n")[1].split("\n  build-signed-artifacts:\n")[0];
  const build = production.split("  build-signed-artifacts:\n")[1].split("\n  smoke-release-dmg:\n")[0];
  const taggedSourceValidation = build
    .split("      - name: Validate exact tagged source\n")[1]
    .split("\n      - name: Validate production credentials are configured\n")[0];
  const smoke = production.split("  smoke-release-dmg:\n")[1].split("\n  publish-release:\n")[0];
  const publish = production.split("  publish-release:\n")[1];
  const productionSecrets = [
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_KEY_P8",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_TEAM_ID",
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  ];
  assert.match(ci, /macos-package-smoke:/);
  assert.match(ci, /npm run smoke:macos-app/);
  assert.doesNotMatch(ci, /Conventional PR title/);
  assert.match(pullRequestPolicy, /pull_request_target:\n\s+types: \[[^\]]*edited[^\]]*\]/);
  assert.doesNotMatch(pullRequestPolicy, /^\s+pull_request:/m);
  assert.doesNotMatch(pullRequestPolicy, /^\s+push:/m);
  assert.match(
    pullRequestPolicy,
    /ref: \$\{\{ github\.sha \}\}/,
  );
  assert.doesNotMatch(
    pullRequestPolicy,
    /github\.event\.pull_request\.(?:base|head)\.sha|github\.head_ref/,
  );
  assert.doesNotMatch(pullRequestPolicy, /secrets\.(APPLE_|TAURI_SIGNING_PRIVATE_KEY)/);
  assert.doesNotMatch(ci, /secrets\.(APPLE_|TAURI_SIGNING_PRIVATE_KEY)/);
  assert.equal(linuxTauriConfig.bundle.createUpdaterArtifacts, false);
  for (const name of productionSecrets) {
    assert.doesNotMatch(releasePleaseJob, new RegExp(name));
    assert.doesNotMatch(preflight, new RegExp(name));
    assert.doesNotMatch(smoke, new RegExp(name));
    assert.doesNotMatch(publish, new RegExp(name));
  }
  assert.match(build, /secrets\.APPLE_CERTIFICATE/);
  assert.match(build, /secrets\.TAURI_SIGNING_PRIVATE_KEY/);
  assert.match(build, /secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD/);
  assert.match(taggedSourceValidation, /node scripts\/release-version\.mjs/);
  assert.doesNotMatch(taggedSourceValidation, /secrets\./);
  assert.match(build, /permissions:\n\s+contents: read/);
  assert.match(smoke, /permissions:\n\s+contents: read/);
  assert.match(publish, /permissions:\n\s+contents: read/);
  assert.match(preflight, /permissions:\n\s+contents: write\n\s+pull-requests: read/);
  assert.match(preflight, /ref: \$\{\{ inputs\.source_sha \}\}\n\s+persist-credentials: false/);
  assert.match(publish, /permission-contents: write/);
  assert.match(publish, /permission-workflows: write/);
  assert.match(publish, /GH_TOKEN: \$\{\{ steps\.release-token\.outputs\.token \}\}/);
  assert.doesNotMatch(smoke, /RELEASE_PLEASE_APP_/);
  assert.match(smoke, /smoke-packaged-macos-dmg\.sh/);
  assert.match(releaseDmg, /verify-updater-signature\.mjs/);
  assert.match(publish, /publish-github-release\.sh/);
  assert.match(publish, /needs: \[preflight, build-signed-artifacts, smoke-release-dmg\]/);
  assert.match(production, /workflow_call:\n\s+inputs:\n\s+source_sha:/);
  assert.match(production, /ref: \$\{\{ inputs\.source_sha \}\}/);
  assert.match(production, /RELEASE_SOURCE_SHA: \$\{\{ inputs\.source_sha \}\}/);
  assert.doesNotMatch(production, /workflow_run|github\.event\.workflow_run/);
  assert.doesNotMatch(production, /group: bridge-release-automation|queue: max/);
  assert.doesNotMatch(production, /push:\n\s+tags:/);
  assert.doesNotMatch(production, /workflow_dispatch/);
  assert.doesNotMatch(releasePlease, /workflow_dispatch/);
  assert.match(productionCall, /needs: release-please/);
  assert.match(productionCall, /permissions:\n\s+contents: write\n\s+pull-requests: read/);
  assert.match(productionCall, /uses: \.\/\.github\/workflows\/release-macos\.yml/);
  assert.match(productionCall, /source_sha: \$\{\{ github\.sha \}\}/);
  assert.match(
    productionCall,
    /release_please_app_client_id: \$\{\{ vars\.RELEASE_PLEASE_APP_CLIENT_ID \}\}/,
  );
  for (const name of [
    "RELEASE_PLEASE_APP_PRIVATE_KEY",
    "APPLE_CERTIFICATE",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY",
    "APPLE_API_KEY_P8",
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  ]) {
    assert.ok(
      productionCall.includes(`${name}: \${{ secrets.${name} }}`),
      `caller does not explicitly pass ${name}`,
    );
  }
  assert.match(releasePlease, /Require deterministic squash-only merges/);
  assert.match(releasePrCheckout, /token: \$\{\{ github\.token \}\}/);
  assert.match(releasePrCheckout, /persist-credentials: false/);
  assert.match(releasePrCheckout, /fetch-depth: 0/);
  assert.match(releasePrCheckout, /path: release-pr/);
  assert.doesNotMatch(releasePrCheckout, /steps\.app-token\.outputs\.token/);
  assert.match(trustedToolsCheckout, /ref: \$\{\{ github\.sha \}\}/);
  assert.match(trustedToolsCheckout, /token: \$\{\{ github\.token \}\}/);
  assert.match(trustedToolsCheckout, /persist-credentials: false/);
  assert.match(trustedToolsCheckout, /path: trusted-release-tools/);
  assert.match(trustedToolsCheckout, /sparse-checkout: scripts\/release-version\.mjs/);
  assert.match(
    lockSync,
    /trusted-release-tools\/scripts\/release-version\.mjs"\s+--root "\$GITHUB_WORKSPACE\/release-pr" --write-locks/,
  );
  assert.doesNotMatch(lockSync, /steps\.app-token|GH_TOKEN/);
  assert.match(lockCommit, /working-directory: release-pr/);
  assert.match(lockCommit, /GH_TOKEN: \$\{\{ steps\.app-token\.outputs\.token \}\}/);
  assert.match(lockCommit, /git config core\.hooksPath \/dev\/null/);
  assert.match(lockCommit, /gh auth setup-git/);
  assert.match(lockCommit, /git push --no-verify origin "HEAD:\$RELEASE_PR_BRANCH"/);
  assert.match(
    lockCommit,
    /git add src-tauri\/Cargo\.lock sidecar\/claude-agent\/package-lock\.json bun\.lock/,
  );
  assert.doesNotMatch(releasePlease, /--write-cargo-lock/);
  assert.match(releasePlease, /rules\/branches\/main/);
  assert.match(releasePlease, /\.type == "pull_request"/);
  assert.match(releasePlease, /\.type == "required_status_checks"/);
  assert.match(releasePlease, /\.context == "Conventional PR title"/);
  assert.match(appToken, /permission-administration: read/);
  assert.ok(
    releasePleaseJob.indexOf("Create repository-scoped release token") <
      releasePleaseJob.indexOf("Require protected pull request inputs"),
    "the Administration-capable App token must exist before branch rules are queried",
  );
  assert.match(protectedInputs, /GH_TOKEN: \$\{\{ steps\.app-token\.outputs\.token \}\}/);
  assert.doesNotMatch(protectedInputs, /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(mergePolicy, /GH_TOKEN: \$\{\{ steps\.app-token\.outputs\.token \}\}/);
  assert.doesNotMatch(mergePolicy, /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(releasePlease, /group: bridge-release-automation\n\s+queue: max/);
  assert.match(releasePlease, /allow_squash_merge, \.allow_merge_commit, \.allow_rebase_merge, \.squash_merge_commit_title, \.squash_merge_commit_message/);
  assert.match(releasePlease, /true,false,false,PR_TITLE,BLANK/);
  for (const [name, workflow] of [
    ["PR title", pullRequestPolicy],
    ["Release Please", releasePlease],
    ["production", production],
  ]) {
    for (const line of workflow.match(/^\s*uses:\s+[^\s]+$/gm) || []) {
      if (/uses:\s+\.\//.test(line)) continue;
      assert.match(line, /@[0-9a-f]{40}$/, `${name} action is not pinned: ${line.trim()}`);
    }
  }
});
