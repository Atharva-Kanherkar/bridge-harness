#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const defaultRoot = fileURLToPath(new URL("../", import.meta.url));
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

function read(path) {
  return readFileSync(path, "utf8");
}

function parseJson(source, label) {
  try {
    return JSON.parse(source);
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`);
  }
}

function escapedRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function tomlSection(source, name) {
  const escaped = escapedRegex(name);
  const header = new RegExp(`^\\[${escaped}\\]\\s*$`, "m").exec(source);
  if (!header) throw new Error(`Missing [${name}] section`);
  const start = header.index + header[0].length;
  const remainder = source.slice(start);
  const nextHeader = /^\[[^\n]+\]\s*$/m.exec(remainder);
  return nextHeader ? remainder.slice(0, nextHeader.index) : remainder;
}

function tomlString(section, key) {
  const escaped = escapedRegex(key);
  const match = section.match(new RegExp(`^${escaped}\\s*=\\s*"([^"]+)"\\s*$`, "m"));
  if (!match) throw new Error(`Missing TOML string ${key}`);
  return match[1];
}

function workspaceMemberPaths(cargoToml) {
  const workspace = tomlSection(cargoToml, "workspace");
  const match = workspace.match(/^members\s*=\s*(\[[^\]]*\])\s*$/m);
  if (!match) throw new Error("Missing Cargo workspace members");
  let members;
  try {
    members = JSON.parse(match[1]);
  } catch (error) {
    throw new Error(`Cargo workspace members are not a simple string array: ${error.message}`);
  }
  if (!Array.isArray(members) || members.some((member) => typeof member !== "string")) {
    throw new Error("Cargo workspace members are not a string array");
  }
  return members;
}

export function cargoWorkspacePackageNames(cargoRoot) {
  const rootManifestPath = join(cargoRoot, "Cargo.toml");
  const rootManifest = read(rootManifestPath);
  const manifests = [
    rootManifestPath,
    ...workspaceMemberPaths(rootManifest).map((member) => join(cargoRoot, member, "Cargo.toml")),
  ];
  return manifests.map((manifestPath) => {
    const manifest = read(manifestPath);
    const packageSection = tomlSection(manifest, "package");
    const inheritsWorkspaceVersion =
      /^version\s*=\s*\{\s*workspace\s*=\s*true\s*\}\s*$/m.test(packageSection) ||
      /^version\.workspace\s*=\s*true\s*$/m.test(packageSection);
    if (!inheritsWorkspaceVersion) {
      throw new Error(`${manifestPath} must inherit workspace.package.version`);
    }
    return tomlString(packageSection, "name");
  });
}

export function cargoLockPackageVersions(lockText, packageNames) {
  const wanted = new Set(packageNames);
  const found = new Map();
  for (const block of lockText.split(/(?=^\[\[package\]\]\s*$)/m)) {
    const name = block.match(/^name\s*=\s*"([^"]+)"\s*$/m)?.[1];
    if (!name || !wanted.has(name)) continue;
    const version = block.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
    if (!version) throw new Error(`Cargo.lock package ${name} has no version`);
    if (found.has(name)) throw new Error(`Cargo.lock contains duplicate workspace package ${name}`);
    found.set(name, version);
  }
  for (const name of wanted) {
    if (!found.has(name)) throw new Error(`Cargo.lock is missing workspace package ${name}`);
  }
  return found;
}

export function updateCargoLockVersions(lockText, packageNames, version) {
  const wanted = new Set(packageNames);
  const seen = new Set();
  const chunks = lockText.split(/(?=^\[\[package\]\]\s*$)/m);
  const updated = chunks.map((block) => {
    const name = block.match(/^name\s*=\s*"([^"]+)"\s*$/m)?.[1];
    if (!name || !wanted.has(name)) return block;
    if (seen.has(name)) throw new Error(`Cargo.lock contains duplicate workspace package ${name}`);
    seen.add(name);
    if (!/^version[ \t]*=[ \t]*"[^"]+"[ \t]*$/m.test(block)) {
      throw new Error(`Cargo.lock package ${name} has no version`);
    }
    return block.replace(/^version[ \t]*=[ \t]*"[^"]+"[ \t]*$/m, `version = "${version}"`);
  });
  for (const name of wanted) {
    if (!seen.has(name)) throw new Error(`Cargo.lock is missing workspace package ${name}`);
  }
  return updated.join("");
}

export function npmPackageLockRootVersions(lockText, packageName) {
  const lock = parseJson(lockText, "sidecar/claude-agent/package-lock.json");
  const rootPackage = lock.packages?.[""];
  if (lock.name !== packageName || rootPackage?.name !== packageName) {
    throw new Error(
      "sidecar/claude-agent/package-lock.json root package does not match package.json",
    );
  }
  if (typeof lock.version !== "string" || typeof rootPackage.version !== "string") {
    throw new Error("sidecar/claude-agent/package-lock.json is missing root version fields");
  }
  return { version: lock.version, rootVersion: rootPackage.version };
}

export function updateNpmPackageLockVersions(lockText, packageName, version) {
  const lock = parseJson(lockText, "sidecar/claude-agent/package-lock.json");
  npmPackageLockRootVersions(lockText, packageName);
  lock.version = version;
  lock.packages[""].version = version;
  return `${JSON.stringify(lock, null, 2)}\n`;
}

function jsonLikeObjectRange(source, property, label) {
  const matches = [
    ...source.matchAll(
      new RegExp(`^[ \\t]*"${escapedRegex(property)}"[ \\t]*:[ \\t]*\\{`, "gm"),
    ),
  ];
  if (matches.length !== 1) {
    throw new Error(`${label} must contain exactly one ${property} object`);
  }
  const start = matches[0].index + matches[0][0].lastIndexOf("{");
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index];
    if (inString) {
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === '"') inString = false;
      continue;
    }
    if (character === '"') inString = true;
    else if (character === "{") depth += 1;
    else if (character === "}" && --depth === 0) {
      return { start, end: index + 1, text: source.slice(start, index + 1) };
    }
  }
  throw new Error(`${label} has an unterminated ${property} object`);
}

function jsonLikeStringProperty(source, property, label) {
  const matches = [
    ...source.matchAll(
      new RegExp(
        `^[ \\t]*"${escapedRegex(property)}"[ \\t]*:[ \\t]*"([^"\\\\]*(?:\\\\.[^"\\\\]*)*)"[ \\t]*,?[ \\t]*$`,
        "gm",
      ),
    ),
  ];
  if (matches.length !== 1) {
    throw new Error(`${label} must contain exactly one ${property} string`);
  }
  return JSON.parse(`"${matches[0][1]}"`);
}

export function bunWorkspacePackageVersion(lockText, workspacePath, packageName) {
  const workspace = jsonLikeObjectRange(lockText, workspacePath, "bun.lock").text;
  if (jsonLikeStringProperty(workspace, "name", `bun.lock ${workspacePath}`) !== packageName) {
    throw new Error(`bun.lock ${workspacePath} package name does not match package.json`);
  }
  return jsonLikeStringProperty(workspace, "version", `bun.lock ${workspacePath}`);
}

export function updateBunWorkspaceVersion(lockText, workspacePath, packageName, version) {
  const workspace = jsonLikeObjectRange(lockText, workspacePath, "bun.lock");
  bunWorkspacePackageVersion(lockText, workspacePath, packageName);
  let replacements = 0;
  const updatedWorkspace = workspace.text.replace(
    /^([ \\t]*"version"[ \\t]*:[ \\t]*)"[^"]*"([ \\t]*,?[ \\t]*)$/gm,
    (_line, prefix, suffix) => {
      replacements += 1;
      return `${prefix}${JSON.stringify(version)}${suffix}`;
    },
  );
  if (replacements !== 1) {
    throw new Error(`bun.lock ${workspacePath} must contain exactly one version string`);
  }
  return `${lockText.slice(0, workspace.start)}${updatedWorkspace}${lockText.slice(workspace.end)}`;
}

function packageBuildVersion(source) {
  const match = source.match(/^pkgver=([^\s#]+)[ \t]+#[ \t]+x-release-please-version[ \t]*$/m);
  if (!match) throw new Error("PKGBUILD has no annotated pkgver");
  return match[1];
}

export function readReleaseVersionState(root = defaultRoot) {
  const packageMetadata = parseJson(read(join(root, "package.json")), "package.json");
  const packageVersion = packageMetadata.version;
  const tauriVersion = parseJson(
    read(join(root, "src-tauri/tauri.conf.json")),
    "src-tauri/tauri.conf.json",
  ).version;
  const browserExtensionVersion = parseJson(
    read(join(root, "browser-extension/manifest.json")),
    "browser-extension/manifest.json",
  ).version;
  const safariExtensionVersion = parseJson(
    read(join(root, "safari-extension/Resources/manifest.json")),
    "safari-extension/Resources/manifest.json",
  ).version;
  const claudePackage = parseJson(
    read(join(root, "sidecar/claude-agent/package.json")),
    "sidecar/claude-agent/package.json",
  );
  const cargoRoot = join(root, "src-tauri");
  const cargoManifest = read(join(cargoRoot, "Cargo.toml"));
  const cargoVersion = tomlString(tomlSection(cargoManifest, "workspace.package"), "version");
  const manifestVersion = parseJson(
    read(join(root, ".release-please-manifest.json")),
    ".release-please-manifest.json",
  )["."];
  const archVersion = packageBuildVersion(read(join(root, "packaging/arch/PKGBUILD")));
  const workspacePackages = cargoWorkspacePackageNames(cargoRoot);
  const cargoLockVersions = cargoLockPackageVersions(
    read(join(cargoRoot, "Cargo.lock")),
    workspacePackages,
  );
  const claudePackageLockVersions = npmPackageLockRootVersions(
    read(join(root, "sidecar/claude-agent/package-lock.json")),
    claudePackage.name,
  );
  const claudeBunWorkspaceVersion = bunWorkspacePackageVersion(
    read(join(root, "bun.lock")),
    "sidecar/claude-agent",
    claudePackage.name,
  );
  return {
    packageVersion,
    tauriVersion,
    browserExtensionVersion,
    safariExtensionVersion,
    claudePackageName: claudePackage.name,
    claudePackageVersion: claudePackage.version,
    cargoVersion,
    manifestVersion,
    archVersion,
    workspacePackages,
    cargoLockVersions,
    claudePackageLockVersions,
    claudeBunWorkspaceVersion,
  };
}

export function verifyReleaseVersions(root = defaultRoot, { checkLocks = true } = {}) {
  const state = readReleaseVersionState(root);
  if (!SEMVER.test(state.packageVersion)) {
    throw new Error(`package.json has invalid semantic version ${state.packageVersion}`);
  }
  for (const [source, version] of [
    ["src-tauri/tauri.conf.json", state.tauriVersion],
    ["browser-extension/manifest.json", state.browserExtensionVersion],
    ["safari-extension/Resources/manifest.json", state.safariExtensionVersion],
    ["sidecar/claude-agent/package.json", state.claudePackageVersion],
    ["src-tauri/Cargo.toml", state.cargoVersion],
    [".release-please-manifest.json", state.manifestVersion],
    ["packaging/arch/PKGBUILD", state.archVersion],
  ]) {
    if (version !== state.packageVersion) {
      throw new Error(`${source} version ${version} does not match package.json ${state.packageVersion}`);
    }
  }
  if (checkLocks) {
    for (const name of state.workspacePackages) {
      const version = state.cargoLockVersions.get(name);
      if (version !== state.packageVersion) {
        throw new Error(
          `src-tauri/Cargo.lock package ${name} version ${version} does not match package.json ${state.packageVersion}`,
        );
      }
    }
    for (const [source, version] of [
      ["sidecar/claude-agent/package-lock.json", state.claudePackageLockVersions.version],
      [
        'sidecar/claude-agent/package-lock.json packages[""]',
        state.claudePackageLockVersions.rootVersion,
      ],
      ['bun.lock workspaces["sidecar/claude-agent"]', state.claudeBunWorkspaceVersion],
    ]) {
      if (version !== state.packageVersion) {
        throw new Error(`${source} version ${version} does not match package.json ${state.packageVersion}`);
      }
    }
  }
  return state;
}

export function syncGeneratedLockVersions(root = defaultRoot) {
  const state = verifyReleaseVersions(root, { checkLocks: false });
  const updates = [
    {
      path: join(root, "src-tauri/Cargo.lock"),
      update: (before) =>
        updateCargoLockVersions(before, state.workspacePackages, state.packageVersion),
    },
    {
      path: join(root, "sidecar/claude-agent/package-lock.json"),
      update: (before) =>
        updateNpmPackageLockVersions(before, state.claudePackageName, state.packageVersion),
    },
    {
      path: join(root, "bun.lock"),
      update: (before) =>
        updateBunWorkspaceVersion(
          before,
          "sidecar/claude-agent",
          state.claudePackageName,
          state.packageVersion,
        ),
    },
  ].map(({ path, update }) => {
    const before = read(path);
    return { path, before, after: update(before) };
  });
  for (const { path, before, after } of updates) {
    if (after !== before) writeFileSync(path, after);
  }
  verifyReleaseVersions(root);
  return updates.some(({ before, after }) => after !== before);
}

function runCli() {
  const args = process.argv.slice(2);
  let root = defaultRoot;
  let writeLocks = false;
  let hasRoot = false;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--write-locks" && !writeLocks) {
      writeLocks = true;
    } else if (arg === "--root" && index + 1 < args.length && !hasRoot) {
      root = resolve(args[index + 1]);
      hasRoot = true;
      index += 1;
    } else {
      throw new Error(
        "Usage: node scripts/release-version.mjs [--write-locks] [--root PATH]",
      );
    }
  }
  const changed = writeLocks
    ? syncGeneratedLockVersions(root)
    : (verifyReleaseVersions(root), false);
  console.log(
    writeLocks
      ? changed
        ? "Synchronized generated lockfile versions."
        : "Generated lockfile versions are already synchronized."
      : "Release versions are synchronized.",
  );
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    runCli();
  } catch (error) {
    console.error(`release-version: ${error.message}`);
    process.exitCode = 1;
  }
}
