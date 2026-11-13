#!/usr/bin/env node
// Scores the drawings `notes::evals::live_drawing_quality` kept.
//
//   node scripts/drawing-bench/score.mjs <results dir> [more dirs...]
//   node scripts/drawing-bench/score.mjs --scene file.excalidraw   (one scene)
//
// Each scene is completed and drawn by the real Excalidraw in headless Chrome,
// through the same code a page's drawing preview runs, at the note column's
// width. The PNG of what the person sees goes to <dir>/png/, the scores to
// <dir>/scores.json, and a summary is printed. Needs Google Chrome; set
// CHROME to its binary elsewhere.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { score, SEVERE } from "./metrics.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

export async function openRenderer() {
  const { createServer } = await import(path.join(root, "node_modules/vite/dist/node/index.js"));
  const server = await createServer({ configFile: path.join(root, "vite.config.ts"), root, logLevel: "error", server: { port: 0, strictPort: false, host: "127.0.0.1" } });
  await server.listen();
  const address = server.httpServer.address();
  const url = `http://127.0.0.1:${address.port}/scripts/drawing-bench/index.html`;
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "draw-bench-chrome-"));
  const debugPort = 9400 + Math.floor(Math.random() * 400);
  const chrome = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`, "--hide-scrollbars", "--window-size=800,1000", "about:blank"], { stdio: "ignore" });
  let targets;
  for (let i = 0; i < 50 && !targets; i++) {
    await sleep(200);
    targets = await fetch(`http://127.0.0.1:${debugPort}/json/list`).then(r => r.json()).catch(() => undefined);
  }
  const page = targets.find(t => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(resolve => { ws.onopen = resolve; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = event => {
    const message = JSON.parse(event.data);
    if (pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); }
  };
  const send = (method, params = {}) => new Promise(resolve => { const n = ++id; pending.set(n, resolve); ws.send(JSON.stringify({ id: n, method, params })); });
  const evaluate = async expression => {
    const { result } = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    return result.result.value;
  };
  await send("Emulation.setDeviceMetricsOverride", { width: 800, height: 1000, deviceScaleFactor: 2, mobile: false });
  await send("Page.navigate", { url });
  for (let i = 0; i < 100; i++) {
    await sleep(200);
    if (await evaluate("typeof window.benchRender === 'function'").catch(() => false)) break;
  }
  const fontsLoaded = await evaluate("window.benchReady");
  if (!fontsLoaded) console.warn("warning: Excalifont did not load; label sizes may be off");
  return {
    evaluate,
    /** A screenshot of `html` laid out at `width` px, for side-by-side sheets. */
    async shoot(html, width, png) {
      await send("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
      const height = await evaluate(`(async () => { document.body.innerHTML = ${JSON.stringify(html)}; await Promise.all([...document.images].map(i => i.decode().catch(() => {}))); return document.body.scrollHeight; })()`);
      const { result } = await send("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width, height, scale: 1 }, captureBeyondViewport: true });
      fs.writeFileSync(png, Buffer.from(result.data, "base64"));
      await send("Emulation.setDeviceMetricsOverride", { width: 800, height: 1000, deviceScaleFactor: 2, mobile: false });
    },
    async render(scene, png) {
      const out = await evaluate(`window.benchRender(${JSON.stringify(scene)})`);
      if (png) {
        const { result } = await send("Page.captureScreenshot", { format: "png", clip: { ...out.rect, scale: 1 } });
        fs.mkdirSync(path.dirname(png), { recursive: true });
        fs.writeFileSync(png, Buffer.from(result.data, "base64"));
      }
      return out.elements;
    },
    async close() {
      ws.close();
      const exited = new Promise(resolve => chrome.once("exit", resolve));
      chrome.kill();
      await exited;
      await server.close();
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    },
  };
}

const pct = (a, b) => (b ? `${Math.round((100 * a) / b)}%` : "-");

export function summarize(rows) {
  const drawn = rows.filter(r => r.score);
  const sum = f => drawn.reduce((total, r) => total + f(r), 0);
  const covPassed = sum(r => r.score.coverage.passed), covTotal = sum(r => r.score.coverage.total);
  return {
    cases: rows.length,
    drawn: drawn.length,
    pass: drawn.filter(r => r.score.pass).length,
    clean: drawn.filter(r => r.score.clean).length,
    coverage: covTotal ? covPassed / covTotal : 0,
    severe: sum(r => r.score.severe),
    crossings: sum(r => r.score.counts.crossing),
    byKind: Object.fromEntries([...SEVERE, "crossing", "dangling"].map(kind => [kind, sum(r => r.score.counts[kind])])),
    minPx: drawn.length ? Math.min(...drawn.map(r => r.score.readability.minPx ?? 99)) : null,
    drawCalls: rows.reduce((total, r) => total + (r.drawCalls ?? 0), 0),
    seconds: rows.reduce((total, r) => total + (r.seconds ?? 0), 0),
  };
}

function printSummary(label, s) {
  console.log(`${label}: drawn ${s.drawn}/${s.cases}, pass ${s.pass}/${s.cases} (${pct(s.pass, s.cases)}), clean ${s.clean}/${s.drawn}, coverage ${(100 * s.coverage).toFixed(1)}%, severe defects ${s.severe}, crossings ${s.crossings}, notes_draw calls ${s.drawCalls}, ${s.seconds}s`);
  console.log(`  severe by kind: ${Object.entries(s.byKind).map(([k, v]) => `${k} ${v}`).join(", ")}`);
}

async function main() {
  const args = process.argv.slice(2);
  if (!args.length) {
    console.error("usage: score.mjs <results dir>... | --scene <file.excalidraw>");
    process.exit(2);
  }
  const renderer = await openRenderer();
  try {
    if (args[0] === "--scene") {
      for (const file of args.slice(1)) {
        const scene = JSON.parse(fs.readFileSync(file, "utf8"));
        const elements = await renderer.render(scene, file.replace(/\.[^.]+$/, "") + ".png");
        const result = score(elements, {});
        console.log(`${path.basename(file)}: ${result.severe} severe, ${result.counts.crossing} crossings, smallest text ${result.readability.minPx?.toFixed(1)}px`);
        for (const d of result.defects) console.log(`  ${d.kind.padEnd(15)} ${d.detail}`);
      }
      return;
    }
    for (const dir of args) {
      const files = fs.readdirSync(dir).filter(f => f.endsWith(".json") && f !== "scores.json" && f !== "judge.json").sort();
      const rows = [];
      for (const file of files) {
        const record = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
        const name = file.replace(/\.json$/, "");
        const row = { id: record.id, run: record.run, holdout: record.holdout, drawCalls: record.drawCalls, seconds: record.seconds, timedOut: record.timedOut, png: null, score: null };
        if (record.scene) {
          const elements = await renderer.render(record.scene, path.join(dir, "png", `${name}.png`));
          const seed = record.seed ? await renderer.render(record.seed, null) : null;
          row.png = `png/${name}.png`;
          row.score = score(elements, record.expect, seed);
        }
        rows.push(row);
        const s = row.score;
        console.log(`${(s ? (s.pass ? "pass" : "FAIL") : "NONE").padEnd(5)} ${(record.holdout ? "[holdout] " : "") + name.padEnd(22)} coverage ${s ? `${s.coverage.passed}/${s.coverage.total}` : "-"}  severe ${s?.severe ?? "-"}  crossings ${s?.counts.crossing ?? "-"}  text ${s?.readability.minPx?.toFixed(1) ?? "-"}px  calls ${record.drawCalls}`);
        for (const d of s?.defects ?? []) if (d.kind !== "dangling") console.log(`        ${d.kind.padEnd(15)} ${d.detail}`);
        for (const c of s?.coverage.checks ?? []) if (!c.ok) console.log(`        missing ${c.kind.padEnd(8)} ${c.detail}`);
      }
      const summary = { all: summarize(rows), dev: summarize(rows.filter(r => !r.holdout)), holdout: summarize(rows.filter(r => r.holdout)) };
      fs.writeFileSync(path.join(dir, "scores.json"), JSON.stringify({ summary, rows }, null, 2));
      console.log("");
      printSummary(`${dir} (all)`, summary.all);
      printSummary("  dev", summary.dev);
      printSummary("  holdout", summary.holdout);
    }
  } finally {
    await renderer.close();
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
