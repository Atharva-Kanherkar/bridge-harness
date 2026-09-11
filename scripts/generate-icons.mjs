#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalizeIcns, verifyIconDirectory } from "./verify-icons.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const result = spawnSync(process.execPath, [
  resolve(root, "node_modules/@tauri-apps/cli/tauri.js"),
  "icon", resolve(root, "assets/bridge-icon.png"),
  "--output", resolve(root, "src-tauri/icons"),
], { cwd: root, stdio: "inherit" });
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const icns = resolve(root, "src-tauri/icons/icon.icns");
writeFileSync(icns, canonicalizeIcns(readFileSync(icns)));
verifyIconDirectory(resolve(root, "src-tauri/icons"));
console.log("Generated and verified Bridge icons from assets/bridge-icon.png.");
