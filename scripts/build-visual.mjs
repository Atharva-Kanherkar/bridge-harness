#!/usr/bin/env node
// Build the MCP Apps view `ui://bridge/visual` into one self-contained HTML
// document: src/mcp-apps/visual/generated/visual.html.
//
// The file is checked in: the frontend imports it with `?raw`, and bridged
// embeds it with `include_str!` to answer `resources/read`, so neither build
// needs the other. `--check` rebuilds into memory and fails if the checked-in
// copy is stale.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "src/mcp-apps/visual/generated/visual.html");
const check = process.argv.includes("--check");

await build({ configFile: join(root, "vite.visual.config.ts"), root });
const dist = join(root, "node_modules/.cache/bridge-visual");
const script = readFileSync(join(dist, "visual.js"), "utf8").replace(/<\/script/gi, "<\\/script");
const style = readFileSync(join(dist, "visual.css"), "utf8").replace(/<\/style/gi, "<\\/style");

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>Bridge visual</title>
<style>${style}</style>
</head>
<body>
<script>${script}</script>
</body>
</html>
`;

if (check) {
  const current = readFileSync(out, "utf8");
  if (current !== html) {
    console.error("src/mcp-apps/visual/generated/visual.html is stale: run `bun run build:visual` and commit the result.");
    process.exit(1);
  }
  console.log("visual.html is current");
} else {
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, html);
  console.log(`wrote ${out} (${(html.length / 1024).toFixed(0)} KiB)`);
}
