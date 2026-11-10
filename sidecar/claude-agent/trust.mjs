// Whether a folder's own `.claude` settings may load.
//
// Project and local settings can run commands (hooks, helper commands, MCP
// servers) and widen permissions, so the `claude` CLI honors them only after
// someone accepts its "Do you trust the files in this folder?" dialog. Driven
// through the Agent SDK, Claude Code treats every folder as trusted, so Bridge
// applies the CLI's rule itself: a folder is trusted when Claude Code's own
// trust record covers it or one of its parents. A Bridge worktree lives under
// the data directory rather than the repository, so it also counts as trusted
// when the main checkout it belongs to does.

import { readFileSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";

/** Where Claude Code keeps its global config, trust records included. */
export function claudeConfigPaths(env = process.env, home = homedir()) {
  return [
    join(env.CLAUDE_CONFIG_DIR || home, ".claude.json"),
    // The legacy file Claude Code still reads when the current one is absent.
    join(env.CLAUDE_CONFIG_DIR || join(home, ".claude"), ".config.json"),
  ];
}

function trustRecords(paths, readFile) {
  for (const path of paths) {
    let text;
    try {
      text = readFile(path, "utf8");
    } catch (error) {
      if (error?.code === "ENOENT") continue;
      return null;
    }
    try {
      const projects = JSON.parse(text)?.projects;
      return projects && typeof projects === "object" ? projects : null;
    } catch {
      return null;
    }
  }
  return null;
}

/** `dir` and every folder above it. */
function* lineage(dir) {
  let current = dir;
  while (true) {
    yield current;
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

/** The main checkout a linked git worktree belongs to, or null for anything else. */
export function mainCheckout(dir, readFile = readFileSync) {
  for (const folder of lineage(dir)) {
    let dotGit;
    try {
      dotGit = readFile(join(folder, ".git"), "utf8");
    } catch (error) {
      // A `.git` directory is a main checkout, not a linked worktree.
      if (error?.code === "EISDIR") return null;
      continue;
    }
    const gitdir = /^gitdir:\s*(.+?)\s*$/m.exec(dotGit)?.[1];
    if (!gitdir) return null;
    const worktreeGitDir = resolve(folder, gitdir);
    let common;
    try {
      common = readFile(join(worktreeGitDir, "commondir"), "utf8").trim();
    } catch {
      // A submodule's `.git` file points at a git dir with no `commondir`.
      return null;
    }
    const commonDir = resolve(worktreeGitDir, common);
    return basename(commonDir) === ".git" ? dirname(commonDir) : null;
  }
  return null;
}

/**
 * Whether Claude Code would honor `cwd`'s project and local settings. Anything
 * missing, relative or unreadable is untrusted.
 */
export function isTrustedFolder(cwd, { env = process.env, home = homedir(), readFile = readFileSync, realpath = realpathSync.native } = {}) {
  if (typeof cwd !== "string" || !isAbsolute(cwd)) return false;
  const projects = trustRecords(claudeConfigPaths(env, home), readFile);
  if (!projects) return false;
  const folders = new Set([resolve(cwd)]);
  try {
    folders.add(realpath(cwd));
  } catch {}
  for (const folder of [...folders]) {
    const main = mainCheckout(folder, readFile);
    if (main) folders.add(main);
  }
  return [...folders].some(folder =>
    [...lineage(folder)].some(dir => projects[dir.normalize("NFC")]?.hasTrustDialogAccepted === true));
}
