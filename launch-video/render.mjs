import { bundle } from "@remotion/bundler";
import {
  selectComposition,
  renderMedia,
  renderStill,
} from "@remotion/renderer";
import { enableTailwind } from "@remotion/tailwind-v4";
import { mkdir, readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
process.chdir(dirname(fileURLToPath(import.meta.url)));
const code = ts.transpileModule(await readFile("src/scenes.ts", "utf8"), {
  compilerOptions: { module: ts.ModuleKind.ESNext },
}).outputText;
const { timeline } = await import(
  "data:text/javascript;base64," + Buffer.from(code).toString("base64")
);
await mkdir("out", { recursive: true });
const serveUrl = await bundle({
  entryPoint: resolve("src/index.tsx"),
  webpackOverride: (config) =>
    enableTailwind({
      ...config,
      resolve: {
        ...config.resolve,
        alias: {
          ...config.resolve?.alias,
          react: resolve("node_modules/react"),
          "react-dom": resolve("node_modules/react-dom"),
        },
      },
    }),
});
const composition = await selectComposition({ serveUrl, id: "BridgeLaunch" });
if (process.argv.includes("--stills")) {
  for (const [i, s] of timeline.entries()) {
    if (process.argv.includes("--quick") && ![0, 1, 4, 6, 15].includes(i))
      continue;
    const frame = s.from + Math.floor(s.duration * 0.72);
    await renderStill({
      serveUrl,
      composition,
      chromiumOptions: { gl: "swangle" },
      frame,
      output: resolve(`out/${String(i + 1).padStart(2, "0")}-${s.id}.png`),
    });
    console.log(`Still ${i + 1}/${timeline.length}: ${s.id}`);
  }
} else {
  let last = -1;
  await renderMedia({
    serveUrl,
    composition,
    chromiumOptions: { gl: "swangle" },
    codec: "h264",
    crf: 18,
    audioCodec: "aac",
    pixelFormat: "yuv420p",
    outputLocation: resolve("out/bridge-launch.mp4"),
    concurrency: 4,
    onProgress: ({ progress }) => {
      const p = Math.floor((progress * 100) / 10) * 10;
      if (p !== last) {
        console.log(`Render ${p}%`);
        last = p;
      }
    },
  });
  console.log("Rendered out/bridge-launch.mp4");
}
