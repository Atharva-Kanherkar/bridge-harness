// Records each feature panel on the landing page as a PNG frame sequence, one frame per
// playback tick, so the README's animated WebPs match the site exactly.
//
//   bun run build && bun run start -p 3187 &
//   bun scripts/record-panels.mjs            # frames land in /tmp/bridge-panels/<id>/
//   python3 scripts/encode-panels.py         # writes ../docs/media/feature-<id>.webp
//
// The page's setInterval is swapped for a manual clock before any script runs, so a frame is
// captured after exactly one tick no matter how slow the screenshot is.
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";

const site = process.env.SITE ?? "http://127.0.0.1:3187/";
const chrome = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const out = "/tmp/bridge-panels";
const port = 9333;

// Ticks per loop, matching each panel's `usePlayback` length.
const all = { parallel: 150, "switch-harness": 150, browser: 160, history: 170, verify: 180, memory: 180, cost: 160 };
// PANELS=browser,memory re-records just those.
const only = process.env.PANELS?.split(",");
const panels = Object.fromEntries(Object.entries(all).filter(([id]) => !only || only.includes(id)));

const browser = spawn(chrome, [`--remote-debugging-port=${port}`, "--headless=new", "--hide-scrollbars", `--user-data-dir=/tmp/bridge-panels-chrome`, "--window-size=1440,1100", "about:blank"], { stdio: "ignore" });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  target = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" }).then(r => r.json()).catch(() => undefined);
}

const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener("open", resolve));
let seq = 0;
const pending = new Map();
socket.addEventListener("message", event => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message.result);
    pending.delete(message.id);
  }
});
const send = (method, params = {}) =>
  new Promise(resolve => {
    const id = ++seq;
    pending.set(id, resolve);
    socket.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async expression => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result.value;

await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 1100, deviceScaleFactor: 2, mobile: false });
await send("Page.addScriptToEvaluateOnNewDocument", {
  source: `
    const clocks = new Map(); let next = 1;
    window.setInterval = fn => { const id = next++; clocks.set(id, fn); return id; };
    window.clearInterval = id => clocks.delete(id);
    window.__tick = () => { for (const fn of [...clocks.values()]) fn(); };
    window.__frame = () => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  `,
});
await send("Page.navigate", { url: site });
await sleep(2500);

for (const [id, length] of Object.entries(panels)) {
  const dir = `${out}/${id}`;
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });

  await evaluate(`document.querySelector('#${id} > div:nth-child(2) > div').scrollIntoView({ block: "center" })`);
  await sleep(600);
  const rect = await evaluate(`(() => { const r = document.querySelector('#${id} > div:nth-child(2) > div').getBoundingClientRect(); return { x: r.x + scrollX, y: r.y + scrollY, width: r.width, height: r.height }; })()`);

  for (let frame = 0; frame < length; frame++) {
    await evaluate("window.__tick(), window.__frame()");
    await sleep(40);
    const { data } = await send("Page.captureScreenshot", { format: "png", clip: { ...rect, scale: 1 }, captureBeyondViewport: true });
    writeFileSync(`${dir}/${String(frame).padStart(4, "0")}.png`, Buffer.from(data, "base64"));
  }
  console.log(`${id}: ${length} frames`);
}

socket.close();
browser.kill();
