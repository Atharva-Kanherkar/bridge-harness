import { readdir, writeFile } from "node:fs/promises";
const css = (await readdir(new URL("./dist/assets/", import.meta.url))).find(
  (f) => f.endsWith(".css"),
);
if (!css)
  throw new Error("Run npm run build before creating the video player.");
await writeFile(
  new URL("./out/watch.html", import.meta.url),
  `<!doctype html>
<html lang="en" class="dark"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Bridge — Launch film</title><link rel="stylesheet" href="../dist/assets/${css}"></head><body class="bg-background text-foreground"><main class="mx-auto max-w-7xl p-8"><p class="font-mono text-sm text-muted-foreground">BRIDGE / LAUNCH FILM</p><h1 class="my-6 text-3xl font-display">Your agents. One control room.</h1><video controls playsinline preload="metadata" poster="01-intro.png" src="bridge-launch.mp4" class="aspect-video w-full rounded-xl border border-border">Your browser does not support video. Open bridge-launch.mp4.</video><p class="mt-6 text-sm text-muted-foreground">102 seconds · 1080p · Original instrumental score</p></main></body></html>`,
);
console.log("Created out/watch.html");
