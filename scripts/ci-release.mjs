// Automatic releases only change versions on a detached release commit. Main
// is never rewritten, and the tag points to the exact source that was tested.
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const stable = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
export const versionFiles = [
  "package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock", "sidecar/claude-agent/package.json",
  "sidecar/claude-agent/package-lock.json", "bun.lock",
  "browser-extension/manifest.json", "safari-extension/Resources/manifest.json",
];
function parts(version) {
  if (!stable.test(version)) throw new Error(`Invalid stable version: ${version}`);
  return version.split(".").map(BigInt);
}
export function compareVersions(a, b) {
  const left = parts(a), right = parts(b);
  for (let i = 0; i < 3; i++) {
    if (left[i] !== right[i]) return left[i] < right[i] ? -1 : 1;
  }
  return 0;
}
export function nextVersion(current, tags) {
  parts(current);
  const versions = tags.filter(tag => tag.startsWith("v") && stable.test(tag.slice(1))).map(tag => tag.slice(1));
  const highest = versions.sort(compareVersions).at(-1);
  // A deliberate minor/major bump in the source is honored. Otherwise advance
  // past every reserved tag, including tags left by interrupted uploads.
  if (!highest || compareVersions(current, highest) > 0) return current;
  const [major, minor, patch] = parts(highest);
  return `${major}.${minor}.${patch + 1n}`;
}
function replaceOne(text, pattern, replacement, label) {
  let count = 0;
  const result = text.replace(pattern, (...args) => { count++; return replacement(...args); });
  if (count !== 1) throw new Error(`Expected one version field in ${label}, found ${count}`);
  return result;
}
export function syncVersions(root, version) {
  parts(version);
  const contents = new Map(versionFiles.map(file => [file, readFileSync(resolve(root, file), "utf8")]));
  for (const file of ["package.json", "src-tauri/tauri.conf.json", "sidecar/claude-agent/package.json", "browser-extension/manifest.json", "safari-extension/Resources/manifest.json"]) {
    contents.set(file, replaceOne(contents.get(file), /^(  "version": ")[^"]+("[,]?)$/gm, (_, a, b) => a + version + b, file));
  }
  const lock = JSON.parse(contents.get("sidecar/claude-agent/package-lock.json"));
  lock.version = lock.packages[""].version = version;
  contents.set("sidecar/claude-agent/package-lock.json", JSON.stringify(lock, null, 2) + "\n");
  contents.set("bun.lock", replaceOne(contents.get("bun.lock"), /("sidecar\/claude-agent": \{\s*"name": "bridge-claude-agent-sidecar",\s*"version": ")[^"]+(")/g, (_, a, b) => a + version + b, "bun.lock"));
  contents.set("src-tauri/Cargo.toml", replaceOne(contents.get("src-tauri/Cargo.toml"), /(\[workspace\.package\]\s*version = ")[^"]+(")/g, (_, a, b) => a + version + b, "Cargo.toml"));
  let cargoLock = contents.get("src-tauri/Cargo.lock");
  for (const name of ["bridge-deck", "bridge-core", "bridge-client", "bridge-protocol", "bridged"]) {
    cargoLock = replaceOne(cargoLock, new RegExp(`(\\[\\[package\\]\\]\\nname = "${name}"\\nversion = ")[^"]+("\\n)(?!source = )`, "g"), (_, a, b) => a + version + b, `Cargo.lock ${name}`);
  }
  contents.set("src-tauri/Cargo.lock", cargoLock);
  // Validate all inputs before writing any file; never resolve new dependencies.
  for (const [file, content] of contents) writeFileSync(resolve(root, file), content);
}

const run = (bin, args) => execFileSync(bin, args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trim();
const git = (...args) => run("git", args);
const gh = (...args) => run("gh", args);
const releases = () => JSON.parse(gh("api", "--paginate", "--slurp", `repos/${process.env.GITHUB_REPOSITORY}/releases?per_page=100`)).flat();
function output(values) {
  for (const [key, value] of Object.entries(values)) appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
}
function assertSource(source) {
  if (!/^[a-f0-9]{40}$/.test(source ?? "")) throw new Error("A full source commit SHA is required");
}
export function prepare() {
  const source = process.env.GITHUB_SHA;
  assertSource(source);
  if (git("status", "--porcelain")) throw new Error("Release preparation requires a clean checkout");
  git("fetch", "origin", "--tags", "--no-recurse-submodules");
  const tags = git("tag", "--list", "v*.*.*").split("\n").filter(tag => stable.test(tag.slice(1)));
  const releaseList = releases(); // Network/auth failures must stop preparation.
  for (const tag of tags) {
    const message = git("show", "-s", "--format=%B", `${tag}^{commit}`);
    if (!message.split("\n").includes(`Bridge-Release-Source: ${source}`)) continue;
    if (git("show", "-s", "--format=%P", `${tag}^{commit}`) !== source) throw new Error(`Invalid release parent for ${tag}`);
    git("checkout", "--detach", tag);
    const version = JSON.parse(readFileSync("src-tauri/tauri.conf.json")).version;
    if (`v${version}` !== tag) throw new Error(`Version mismatch in ${tag}`);
    output({ tag, version, commit: git("rev-parse", "HEAD"), published: releaseList.some(r => r.tag_name === tag && !r.draft) });
    return;
  }
  git("checkout", "--detach", source);
  const version = nextVersion(JSON.parse(readFileSync("src-tauri/tauri.conf.json")).version, tags);
  syncVersions(process.cwd(), version);
  git("add", "--", ...versionFiles);
  // Allow an empty version commit when a PR already supplied the next version.
  // Its parent + trailer still provide an unambiguous idempotency key.
  git("-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com", "commit", "--allow-empty", "-m", `chore: release macOS ${version}`, "-m", `Bridge-Release-Source: ${source}`);
  output({ version, tag: `v${version}`, commit: git("rev-parse", "HEAD"), published: false });
}

// Injectable commands let tests prove that failed uploads cannot publish, and
// that retries reuse drafts without modifying already-public release assets.
export function publish({ tag, commit, source, dmg, command = gh, gitCommand = git, releaseList = releases() }) {
  assertSource(source);
  assertSource(commit);
  parts(tag.slice(1));
  if (tag[0] !== "v") throw new Error("Invalid release tag");
  if (gitCommand("rev-parse", "HEAD") !== commit || gitCommand("show", "-s", "--format=%P", commit) !== source) throw new Error("Release does not match the prepared source commit");
  if (gitCommand("diff", "HEAD", "--", ...versionFiles)) throw new Error("Build changed the prepared version files");
  const existing = releaseList.find(r => r.tag_name === tag);
  if (existing && !existing.draft) return;
  const checksum = `${dmg}.sha256`;
  const digest = createHash("sha256").update(readFileSync(dmg)).digest("hex");
  if (readFileSync(checksum, "utf8").trim() !== `${digest}  ${basename(dmg)}`) throw new Error("DMG checksum mismatch");
  const localTags = gitCommand("tag", "--list", tag);
  if (localTags) {
    if (gitCommand("rev-parse", `${tag}^{commit}`) !== commit) throw new Error("Release tag already belongs to another commit");
  } else gitCommand("tag", tag, commit);
  // A competing tag creation fails here, before creating/updating any release.
  gitCommand("push", "origin", `refs/tags/${tag}`);
  if (!existing) command("release", "create", tag, "--verify-tag", "--draft", "--title", `Bridge v${tag.slice(1)}`, "--generate-notes", "--notes", `Signed and notarized macOS release. Source commit: ${source}.\n\nClaude requires Node 18 or later on PATH.`);
  command("release", "upload", tag, dmg, checksum, "--clobber");
  const latest = !releaseList.some(r => !r.draft && !r.prerelease && stable.test(r.tag_name.slice(1)) && compareVersions(r.tag_name.slice(1), tag.slice(1)) > 0);
  command("release", "edit", tag, "--draft=false", `--latest=${latest}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.env.GITHUB_REF !== "refs/heads/main") throw new Error("Automatic public releases only run from main");
  if (process.argv[2] === "prepare") prepare();
  else if (process.argv[2] === "publish") {
    const arch = { arm64: "aarch64", x64: "x64" }[process.arch];
    if (!arch) throw new Error("Unsupported release architecture");
    publish({ tag: process.env.RELEASE_TAG, commit: process.env.RELEASE_COMMIT, source: process.env.GITHUB_SHA, dmg: `src-tauri/target/release/bundle/dmg/Bridge_${process.env.RELEASE_TAG.slice(1)}_${arch}.dmg` });
  } else throw new Error("Usage: node scripts/ci-release.mjs prepare|publish");
}
