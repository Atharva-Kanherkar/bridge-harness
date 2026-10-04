#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalizeIcns, verifyIconDirectory } from "./verify-icons.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
// Render every app and website icon from the same vector B.
const icon = resolve(root, "assets/bridge-icon.svg");
const artwork = readFileSync(icon, "utf8");
const mark = artwork.match(/<path\b[^>]*\bd="([^"]+)"/);
if (!mark) throw new Error("Bridge icon must contain its canonical B path");
mkdirSync(resolve(root, "src/brand"), { recursive: true });
writeFileSync(resolve(root, "src/brand/bridgeIcon.ts"),
  `// Derived from assets/bridge-icon.svg. Refresh with bun run generate:icons.\nexport const BRIDGE_ICON_PATH = ${JSON.stringify(mark[1])};\n`);
writeFileSync(resolve(root, "src-tauri/bridge-menu-bar/swift/BridgeIconArtwork.swift"),
  `// Derived from assets/bridge-icon.svg. Refresh with bun run generate:icons.\nenum BridgeIconArtwork {\n    static let svg = #"""\n    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="4.5 4 24 24">\n      <path fill="black" fill-rule="evenodd" d="${mark[1]}"/>\n    </svg>\n    """#\n}\n`);
const result = spawnSync(process.execPath, [
  resolve(root, "node_modules/@tauri-apps/cli/tauri.js"),
  "icon", icon,
  "--output", resolve(root, "src-tauri/icons"),
], { cwd: root, stdio: "inherit" });
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const icns = resolve(root, "src-tauri/icons/icon.icns");
writeFileSync(icns, canonicalizeIcns(readFileSync(icns)));
verifyIconDirectory(resolve(root, "src-tauri/icons"));
copyFileSync(resolve(root, "src-tauri/icons/icon.png"), resolve(root, "assets/bridge-icon.png"));
copyFileSync(resolve(root, "src-tauri/icons/icon.png"), resolve(root, "landing/app/icon.png"));
copyFileSync(resolve(root, "src-tauri/icons/icon.ico"), resolve(root, "landing/app/favicon.ico"));
console.log("Generated and verified Bridge app and website icons from assets/bridge-icon.svg.");
