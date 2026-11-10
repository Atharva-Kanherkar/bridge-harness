import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { claudeConfigPaths, isTrustedFolder, mainCheckout } from "../trust.mjs";

function scratch(t) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "bridge-trust-")));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const home = join(root, "home");
  mkdirSync(home);
  return { root, home };
}

function trust(home, folders, file = ".claude.json") {
  const projects = Object.fromEntries(Object.entries(folders).map(([path, accepted]) => [path, { hasTrustDialogAccepted: accepted }]));
  writeFileSync(join(home, file), JSON.stringify({ projects }));
}

function git(cwd, ...args) {
  execFileSync("git", ["-c", "user.email=t@example.com", "-c", "user.name=t", ...args], { cwd, stdio: "ignore" });
}

function repo(path) {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q");
  writeFileSync(join(path, "README.md"), "hi\n");
  git(path, "add", "-A");
  git(path, "commit", "-qm", "init");
}

test("a folder Claude Code has not trusted is untrusted", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  mkdirSync(project);
  trust(home, { [join(root, "other")]: true, [project]: false });
  assert.equal(isTrustedFolder(project, { env: {}, home }), false);
});

test("a folder trusted in Claude Code is trusted, and so are its subfolders", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  mkdirSync(join(project, "packages", "app"), { recursive: true });
  trust(home, { [project]: true });
  assert.equal(isTrustedFolder(project, { env: {}, home }), true);
  assert.equal(isTrustedFolder(join(project, "packages", "app"), { env: {}, home }), true);
});

test("trust does not leak to a sibling that shares a name prefix", t => {
  const { root, home } = scratch(t);
  mkdirSync(join(root, "project"));
  mkdirSync(join(root, "project-evil"));
  trust(home, { [join(root, "project")]: true });
  assert.equal(isTrustedFolder(join(root, "project-evil"), { env: {}, home }), false);
});

test("missing, corrupt or project-less config, and a relative or absent cwd, are untrusted", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  mkdirSync(project);
  assert.equal(isTrustedFolder(project, { env: {}, home }), false);
  writeFileSync(join(home, ".claude.json"), "{not json");
  assert.equal(isTrustedFolder(project, { env: {}, home }), false);
  writeFileSync(join(home, ".claude.json"), JSON.stringify({ numStartups: 3 }));
  assert.equal(isTrustedFolder(project, { env: {}, home }), false);
  trust(home, { [project]: true });
  assert.equal(isTrustedFolder("project", { env: {}, home }), false);
  assert.equal(isTrustedFolder(undefined, { env: {}, home }), false);
});

test("CLAUDE_CONFIG_DIR moves the trust record, as it does for Claude Code", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  const configDir = join(root, "claude-config");
  mkdirSync(project);
  mkdirSync(configDir);
  trust(home, { [project]: true });
  trust(configDir, {});
  assert.equal(isTrustedFolder(project, { env: { CLAUDE_CONFIG_DIR: configDir }, home }), false);
  trust(configDir, { [project]: true });
  assert.equal(isTrustedFolder(project, { env: { CLAUDE_CONFIG_DIR: configDir }, home }), true);
  assert.deepEqual(claudeConfigPaths({ CLAUDE_CONFIG_DIR: configDir }, home), [join(configDir, ".claude.json"), join(configDir, ".config.json")]);
});

test("the legacy config file is read only when the current one is absent", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  mkdirSync(project);
  mkdirSync(join(home, ".claude"));
  trust(join(home, ".claude"), { [project]: true }, ".config.json");
  assert.equal(isTrustedFolder(project, { env: {}, home }), true);
  trust(home, {});
  assert.equal(isTrustedFolder(project, { env: {}, home }), false);
});

test("a Bridge worktree of a trusted checkout is trusted; one of an untrusted checkout is not", t => {
  const { root, home } = scratch(t);
  const trusted = join(root, "repos", "mine");
  const untrusted = join(root, "repos", "theirs");
  repo(trusted);
  repo(untrusted);
  // Bridge keeps worktrees under its data directory, outside the repository.
  const mineWorktree = join(root, "data", "worktrees", "mine-task");
  const theirsWorktree = join(root, "data", "worktrees", "theirs-task");
  git(trusted, "worktree", "add", "-q", "-b", "task", mineWorktree);
  git(untrusted, "worktree", "add", "-q", "-b", "task", theirsWorktree);
  mkdirSync(join(mineWorktree, "src"));
  trust(home, { [trusted]: true });

  assert.equal(mainCheckout(mineWorktree), trusted);
  assert.equal(mainCheckout(join(mineWorktree, "src")), trusted);
  assert.equal(mainCheckout(trusted), null);
  assert.equal(isTrustedFolder(mineWorktree, { env: {}, home }), true);
  assert.equal(isTrustedFolder(join(mineWorktree, "src"), { env: {}, home }), true);
  assert.equal(isTrustedFolder(theirsWorktree, { env: {}, home }), false);
});

test("a symlinked path is trusted through the folder it resolves to", t => {
  const { root, home } = scratch(t);
  const project = join(root, "project");
  mkdirSync(project);
  execFileSync("ln", ["-s", project, join(root, "alias")]);
  trust(home, { [project]: true });
  assert.equal(isTrustedFolder(join(root, "alias"), { env: {}, home }), true);
});
