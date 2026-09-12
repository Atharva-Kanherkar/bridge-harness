import assert from "node:assert/strict";
import { readFile, access } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
process.chdir(dirname(fileURLToPath(import.meta.url)));
const code = ts.transpileModule(await readFile("src/scenes.ts", "utf8"), {
  compilerOptions: { module: ts.ModuleKind.ESNext },
}).outputText;
const { timeline, DURATION, FPS } = await import(
  "data:text/javascript;base64," + Buffer.from(code).toString("base64")
);
assert.equal(timeline[0].from, 0);
assert.equal(new Set(timeline.map((s) => s.id)).size, timeline.length);
for (const [i, scene] of timeline.entries()) {
  assert(scene.duration >= 6 * FPS);
  if (i)
    assert.equal(scene.from, timeline[i - 1].from + timeline[i - 1].duration);
  for (const source of scene.source.split("; "))
    await access(resolve("..", source));
  if (scene.image) await access(resolve("public/media", scene.image));
}
assert.equal(timeline.at(-1).from + timeline.at(-1).duration, DURATION);
await access("public/score.wav");
console.log(
  `PASS: ${timeline.length} scenes, ${DURATION / FPS}s; continuous timeline, unique chapters, all evidence and assets present.`,
);
