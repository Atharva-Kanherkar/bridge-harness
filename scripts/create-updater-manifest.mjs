#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

export function createUpdaterManifest({
  version,
  tag,
  arch,
  signature,
  repository,
  updaterName,
  notes,
  pubDate,
}) {
  if (!VERSION.test(version)) throw new Error(`Invalid updater version: ${version}`);
  if (tag !== `v${version}`) throw new Error(`Updater tag ${tag} does not match v${version}`);
  if (!new Set(["aarch64", "x64"]).has(arch)) {
    throw new Error(`Unsupported updater architecture: ${arch}`);
  }
  if (!REPOSITORY.test(repository)) throw new Error(`Invalid GitHub repository: ${repository}`);
  const expectedUpdaterName = `Bridge_${version}_${arch}.app.tar.gz`;
  if (basename(updaterName) !== updaterName || updaterName !== expectedUpdaterName) {
    throw new Error(`Invalid updater artifact name: ${updaterName}`);
  }
  if (typeof signature !== "string" || !signature.trim()) {
    throw new Error("Updater signature is empty");
  }
  if (!Number.isFinite(Date.parse(pubDate))) throw new Error(`Invalid publication date: ${pubDate}`);
  const updaterPlatformArch = arch === "x64" ? "x86_64" : arch;
  return {
    version,
    notes: typeof notes === "string" && notes.trim() ? notes.trim() : "See CHANGELOG.md for changes.",
    pub_date: pubDate,
    platforms: {
      [`darwin-${updaterPlatformArch}`]: {
        signature: signature.trim(),
        url: `https://github.com/${repository}/releases/download/${tag}/${updaterName}`,
      },
    },
  };
}

function value(name) {
  const index = process.argv.indexOf(`--${name}`);
  if (index < 0 || index + 1 >= process.argv.length) {
    throw new Error(`Missing --${name}`);
  }
  return process.argv[index + 1];
}

function runCli() {
  const output = value("output");
  const manifest = createUpdaterManifest({
    version: value("version"),
    tag: value("tag"),
    arch: value("arch"),
    signature: readFileSync(value("signature-file"), "utf8"),
    repository: value("repository"),
    updaterName: value("updater-name"),
    notes: readFileSync(value("notes-file"), "utf8"),
    pubDate: value("pub-date"),
  });
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    runCli();
  } catch (error) {
    console.error(`create-updater-manifest: ${error.message}`);
    process.exitCode = 1;
  }
}
