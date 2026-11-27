import { test } from "node:test";
import assert from "node:assert/strict";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = fileURLToPath(new URL("../../", import.meta.url));

function executable(path, body) {
  writeFileSync(path, body);
  chmodSync(path, 0o755);
}

// A checkout with the real ad-hoc script, a stubbed smoke build and stubbed
// macOS tools, so the packaging logic runs on any host.
function fixture(t, { os = "Darwin", bundleVersion = "1.2.3" } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "bridge-adhoc-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, "scripts"));
  mkdirSync(join(dir, "src-tauri"));
  mkdirSync(join(dir, "bin"));
  cpSync(join(root, "scripts/release-dmg-adhoc.sh"), join(dir, "scripts/release-dmg-adhoc.sh"));
  writeFileSync(join(dir, "src-tauri/tauri.conf.json"), '{"version":"1.2.3"}');
  executable(join(dir, "scripts/build-macos-smoke.sh"), `#!/bin/sh
mkdir -p "$PWD/src-tauri/target/release/bundle/macos/Bridge.app/Contents"
printf built > "$PWD/build-ran"
`);
  executable(join(dir, "bin/uname"), `#!/bin/sh
case "$1" in -s) echo ${os} ;; -m) echo arm64 ;; esac
`);
  executable(join(dir, "bin/plutil"), `#!/bin/sh\necho ${bundleVersion}\n`);
  executable(join(dir, "bin/ditto"), '#!/bin/sh\ncp -R "$1" "$2"\n');
  // create: write the source folder's listing as the "image"; verify: succeed.
  executable(join(dir, "bin/hdiutil"), `#!/bin/sh
if [ "$1" = create ]; then
  for out; do :; done
  while [ "$#" -gt 0 ]; do [ "$1" = -srcfolder ] && src=$2; shift; done
  (cd "$src" && ls -A && readlink Applications && cat "How to open Bridge.txt") > "$out"
fi
`);
  const run = () => spawnSync("/bin/sh", [join(dir, "scripts/release-dmg-adhoc.sh")], {
    cwd: dir,
    env: { ...process.env, PATH: `${join(dir, "bin")}:${process.env.PATH}`, TMPDIR: dir },
    encoding: "utf8",
  });
  return { dir, run, dmgDir: join(dir, "src-tauri/target/release/bundle/dmg") };
}

test("the ad-hoc DMG refuses to build off macOS", (t) => {
  const { dir, run, dmgDir } = fixture(t, { os: "Linux" });
  const out = run();
  assert.notEqual(out.status, 0);
  assert.match(out.stderr, /macOS is required/);
  assert.equal(existsSync(join(dir, "build-ran")), false);
  assert.equal(existsSync(dmgDir), false);
});

test("the ad-hoc DMG refuses a bundle built from another version", (t) => {
  const { run, dmgDir } = fixture(t, { bundleVersion: "1.2.2" });
  const out = run();
  assert.notEqual(out.status, 0);
  assert.match(out.stderr, /Bridge.app is 1\.2\.2 but this checkout is 1\.2\.3/);
  assert.equal(existsSync(dmgDir), false);
});

test("the ad-hoc DMG is named as ad-hoc and ships the opening instructions", (t) => {
  const { run, dmgDir } = fixture(t);
  const out = run();
  assert.equal(out.status, 0, out.stderr);
  const dmg = join(dmgDir, "Bridge_1.2.3_aarch64_adhoc.dmg");
  const image = readFileSync(dmg, "utf8");
  assert.match(image, /^Bridge\.app$/m);
  assert.match(image, /^Applications$/m);
  assert.match(image, /^\/Applications$/m);
  assert.match(image, /\/usr\/bin\/xattr -cr \/Applications\/Bridge\.app/);
  assert.match(image, /Open Anyway/);
  assert.match(readFileSync(`${dmg}.sha256`, "utf8"), /^[0-9a-f]{64}  Bridge_1\.2\.3_aarch64_adhoc\.dmg\n$/);
  assert.match(out.stdout, /not notarized/);
});
