#!/usr/bin/env node
// A scripted fake user for "Move to Bridge". It drives the real app in a
// browser tab attached to a real `bridged` (see docs/web-client.md), the way a
// person would: open Settings, find their tools, press Import once, watch the
// progress, leave the page while it works, get told when it is done, open the
// imported chats, then import again to see that nothing lands twice. Every
// step asserts what is on screen; any mismatch exits non-zero. It records the
// run as screenshots, an MP4 and a GIF.
//
//   HOME=<fake home> bridged --data-dir <dir> --listen 127.0.0.1:4391 --allowed-origin http://127.0.0.1:1471
//   bunx vite --port 1471 --strictPort
//   LINK="$(node scripts/web-link.mjs --data-dir <dir> --daemon ws://127.0.0.1:4391 --app http://127.0.0.1:1471 | tail -1)" \
//     OUT=/tmp/fake-user node scripts/e2e/import-fake-user.mjs
//
// The fake home is built by scripts/e2e/import-fake-home.py.

import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const LINK = process.env.LINK;
const OUT = process.env.OUT ?? "/tmp/import-fake-user";
const PORT = Number(process.env.CDP_PORT ?? 9377);
const RECORD = process.env.RECORD !== "0";
// How many generated chats the fake home holds (import-fake-home.py), plus 5 hand-made.
const CHATS = Number(process.env.IMPORT_FAKE_CHATS ?? 600) + 5;
if (!LINK) throw new Error("Set LINK to a daemon link from scripts/web-link.mjs");

rmSync(OUT, { recursive: true, force: true });
mkdirSync(join(OUT, "frames"), { recursive: true });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const log = (...parts) => console.log(new Date().toISOString().slice(11, 19), ...parts);

const chrome = spawn(CHROME, ["--headless=new", `--remote-debugging-port=${PORT}`, "--window-size=1440,900", `--user-data-dir=/tmp/import-fake-user-${Date.now()}`, "--no-first-run", "about:blank"], { stdio: "ignore" });
let targets;
for (let i = 0; i < 60 && !targets; i++) { try { targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json(); } catch { await sleep(250); } }
const socket = new WebSocket(targets.find(target => target.type === "page").webSocketDebuggerUrl);
await new Promise(resolve => { socket.onopen = resolve; });
let nextId = 0;
const pending = new Map();
const frames = [];
// The first frame of the transfer clip: when the progress card showed.
let transferFrom = 0;
socket.onmessage = event => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); return; }
  if (message.method === "Page.screencastFrame") {
    const { data, metadata, sessionId } = message.params;
    const file = join(OUT, "frames", `${String(frames.length).padStart(5, "0")}.jpg`);
    writeFileSync(file, Buffer.from(data, "base64"));
    frames.push({ file, at: metadata.timestamp });
    send("Page.screencastFrameAck", { sessionId });
  }
};
const send = (method, params = {}) => new Promise(resolve => { const id = ++nextId; pending.set(id, resolve); socket.send(JSON.stringify({ id, method, params })); });
const js = async expression => {
  const reply = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.result?.exceptionDetails) throw new Error(`page error: ${JSON.stringify(reply.result.exceptionDetails).slice(0, 300)}`);
  return reply.result?.result?.value;
};
const text = () => js("document.body.innerText");
const shots = [];
const shot = async name => {
  const { result } = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(OUT, `${name}.png`), Buffer.from(result.data, "base64"));
  shots.push(name);
  log("screenshot", name);
};
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  log(ok ? "PASS" : "FAIL", name, detail);
  if (!ok) throw new Error(`Fake user check failed: ${name} ${detail}`);
};
async function waitFor(description, predicate, timeout = 20000) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    if (await js(predicate)) return true;
    await sleep(250);
  }
  const slug = description.replace(/\W+/g, "-");
  await shot(`timeout-${slug}`);
  writeFileSync(join(OUT, `timeout-${slug}.txt`), await text());
  throw new Error(`Timed out waiting for ${description}`);
}
async function caption(line) {
  await js(`(() => {
    let el = document.getElementById("fake-user-caption");
    if (!el) { el = document.createElement("div"); el.id = "fake-user-caption";
      el.setAttribute("style", "position:fixed;left:50%;bottom:18px;transform:translateX(-50%);z-index:2147483647;padding:8px 14px;border-radius:999px;background:rgba(20,20,20,.88);color:#fff;font:500 13px system-ui;pointer-events:none;box-shadow:0 4px 20px rgba(0,0,0,.25)");
      document.body.appendChild(el); }
    el.textContent = ${JSON.stringify(line)};
  })()`);
  await sleep(900);
}
/** Click the first visible element of `selector` whose text includes `label`, like a person would. */
async function click(label, selector = "button, a, [role=button], [role=switch]") {
  const found = await js(`(() => {
    const items = [...document.querySelectorAll(${JSON.stringify(selector)})].filter(el => el.offsetParent !== null);
    // What a person reads first: visible text, then the accessible name.
    // Innermost match wins, the way a pointer lands on the text itself.
    const byText = items.filter(el => (el.textContent ?? "").includes(${JSON.stringify(label)})).sort((a, b) => (a.textContent ?? "").length - (b.textContent ?? "").length);
    const el = byText[0] ?? items.find(el => (el.getAttribute("aria-label") ?? "").includes(${JSON.stringify(label)}));
    if (!el) return false;
    el.scrollIntoView({ block: "center" });
    const box = el.getBoundingClientRect();
    let dot = document.getElementById("fake-user-cursor");
    if (!dot) { dot = document.createElement("div"); dot.id = "fake-user-cursor";
      dot.setAttribute("style", "position:fixed;width:18px;height:18px;margin:-9px 0 0 -9px;border-radius:50%;background:rgba(255,255,255,.35);border:2px solid #111;z-index:2147483647;pointer-events:none;transition:left .35s,top .35s");
      document.body.appendChild(dot); }
    dot.style.left = (box.left + box.width / 2) + "px"; dot.style.top = (box.top + box.height / 2) + "px";
    setTimeout(() => el.click(), 380);
    return true;
  })()`);
  if (!found) throw new Error(`Could not find "${label}" to click`);
  await sleep(1200);
}

try {
  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  if (RECORD) await send("Page.startScreencast", { format: "jpeg", quality: 70, maxWidth: 1440, maxHeight: 900, everyNthFrame: 1 });

  log("open the app attached to the real daemon");
  await send("Page.navigate", { url: LINK });
  await waitFor("the app to load", `document.body.innerText.length > 50`);
  await sleep(2500);
  // The fake user already uses Bridge with Claude Code. A fake home has no
  // signed-in agent, so finish setup the way the wizard does, then reload.
  if (await js(`document.body.innerText.includes("Choose agents")`)) {
    const done = await js(`(async () => {
      const { bridgeApi } = await import("/src/api.ts");
      const { recommendedProfileDrafts } = await import(${JSON.stringify(process.env.PROFILES_MODULE ?? "/src/modelProfiles.ts")});
      const health = await bridgeApi.health();
      const setup = await bridgeApi.saveModelProfiles(recommendedProfileDrafts(health.adapters.filter(a => a.id === "claude"), { allowAvailableFallback: true }));
      return setup.complete;
    })()`);
    check("Setup completes for a returning Claude Code user", done === true);
    await send("Page.navigate", { url: LINK });
    await waitFor("the app to load", `!!document.querySelector('button[aria-label^="Open settings for"]')`, 30000);
    await sleep(1500);
  }

  // A development build shows a code-signing banner; a person closes it.
  await js(`(() => {
    let node = [...document.querySelectorAll("*")].find(el => el.childElementCount === 0 && /ad-hoc signed/.test(el.textContent ?? ""));
    for (let i = 0; node && i < 5; i++, node = node.parentElement) {
      const close = node.querySelector("button");
      if (close) { close.click(); return true; }
    }
    return false;
  })()`);
  await caption("Open Settings, then Chat history, then Import");
  await click("Open settings for");
  await click("Chat history");
  await waitFor("the Chat history page", `!!document.querySelector("[data-settings-column]") && document.querySelector("[data-settings-column]").innerText.includes("Archived chats")`);
  const history = await js(`document.querySelector("[data-settings-column]").innerText`);
  check("Chat history has its own place in Settings, with Import and Archived chats", history.includes("Chat history") && history.includes("Import") && history.includes("Archived chats"));
  await shot("00-chat-history-dark");
  await click("Import", "main button, [data-settings-column] button");
  await waitFor("the tools list", `document.body.innerText.includes("Found on this Mac") && !document.body.innerText.includes("Looking for other tools")`);
  await caption("Bridge finds the tools already on this Mac, without reading any chat");
  const found = await text();
  check("Claude Code is found with its chats, projects, setup and the background run left out", new RegExp(`${CHATS.toLocaleString("en-US")} chats in 6 projects · 1 setup item · \\d+ background runs? left out`).test(found), found.slice(found.indexOf("Found on this Mac"), found.indexOf("Found on this Mac") + 700).replace(/\n+/g, " | "));
  check("Conductor is found by its worktrees and says its chats come with Claude Code", found.includes("3 workspaces in 2 repos") && found.includes("Conductor runs Claude Code"));
  check("Codex is found with its own section, mark and Import button", found.includes("Codex keeps using its own memory")
    && await js(`!!document.querySelector('[data-import-tool="codex"] [data-tool-mark] svg') && !!document.querySelector('[data-import-tool="codex"] button')`));
  check("OpenCode is found with its own section, mark and Import button, its background run apart", found.includes("OpenCode keeps its own memory and setup")
    && /1 chat in 1 project · 1 background run left out/.test(await js(`document.querySelector('[data-import-tool="opencode"]').innerText`))
    && await js(`!!document.querySelector('[data-import-tool="opencode"] [data-tool-mark] svg') && !!document.querySelector('[data-import-tool="opencode"] button')`));
  check("Tools that are not installed say so", found.includes("Not found on this Mac"));
  await shot("01-found-dark");

  // The generated chats plus 5 hand-made ones, the Explore subagent and
  // CLAUDE.md: files read = chats + 1, items written = chats + 2 (the subagent
  // comes out of its parent's file). Claude Code keeps its own memory, so the
  // memory file stays where it is.
  const EXPECTED = CHATS + 2;
  await caption("One click: Import");
  await click("Import", "button");
  await waitFor("the progress card", `!!document.querySelector('section[aria-label="Import progress"]')`, 10000);
  // Centre the card first so it stays put for the clip and the close-ups;
  // the clip starts with the frame on screen once it is there.
  await js(`document.querySelector('section[aria-label="Import progress"]').scrollIntoView({ block: "center" })`);
  await sleep(300);
  transferFrom = frames.length;
  check("Import starts at once and shows its progress", true);
  check("The Import button waits while the import runs", await js(`[...document.querySelectorAll("button")].some(b => b.textContent.includes("Importing") && b.disabled)`));
  await waitFor("the first batches", `/\\d+ of \\d+/.test(document.querySelector('section[aria-label="Import progress"]')?.innerText ?? "")`, 30000);
  await sleep(700);
  await shot("02-progress-dark");
  // A close-up of the transfer card, sharp, twice: the cards are mid-flight.
  // A few seconds in, so the time left has a pace to go on.
  await sleep(3500);
  await js(`(() => { const dot = document.getElementById("fake-user-cursor"); if (dot) dot.style.opacity = "0"; })()`);
  const rect = await js(`(() => { const r = document.querySelector('section[aria-label="Import progress"]').getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; })()`);
  writeFileSync(join(OUT, "transfer-rect.json"), JSON.stringify(rect));
  for (const name of ["02b-transfer-close", "02c-transfer-close"]) {
    const { result } = await send("Page.captureScreenshot", { format: "png", clip: { ...rect, scale: 2 } });
    writeFileSync(join(OUT, `${name}.png`), Buffer.from(result.data, "base64"));
    shots.push(`${name}.png`);
    await sleep(450);
  }
  await js(`(() => { const dot = document.getElementById("fake-user-cursor"); if (dot) dot.style.opacity = "1"; })()`);
  check("The transfer shows chats moving from Claude Code to Bridge", await js(`document.querySelectorAll('[data-transfer="running"] .import-transfer-card').length === 6`));
  check("It says how long is left", /left|Working out/.test(await js(`document.querySelector("[data-import-time-left]")?.textContent ?? ""`)));
  const card = await js(`document.querySelector('section[aria-label="Import progress"]').innerText`);
  check("The card counts files and says what came over", new RegExp(`of ${(CHATS + 1).toLocaleString("en-US")}`).test(card) && /came over/.test(card), card.replace(/\n+/g, " | "));
  check("The card says the page can be left", card.includes("You can leave this page"));

  await caption("Leave the page while it works");
  await click("Data & storage");
  await waitFor("Bridge to say the import is done", `document.body.innerText.includes("Import finished")`, 120000);
  const toast = await text();
  check("Bridge says when it is done, on whatever page you are on", toast.includes(`${EXPECTED.toLocaleString("en-US")} items came over from Claude Code`), toast.slice(toast.indexOf("Import finished"), toast.indexOf("Import finished") + 120).replace(/\n+/g, " | "));
  await shot("03-finished-toast-dark");

  await caption("Open the summary from the notice");
  await click("Import finished: open the import summary", "button[aria-label]");
  await waitFor("the summary", `!!document.querySelector('[data-import-result="imported"]')`, 10000);
  const imported = Number((await js(`document.querySelector('[data-import-result="imported"]')?.textContent`)).replace(/,/g, ""));
  const skippedFirst = Number(await js(`document.querySelector('[data-import-result="skipped"]')?.textContent`));
  const summary = await text();
  check("Every chat, the subagent and CLAUDE.md came over", imported === EXPECTED, `imported=${imported}`);
  check("Nothing was already in Bridge on the first import", skippedFirst === 0, `skipped=${skippedFirst}`);
  // The fixture's temp-folder run, plus the probes Bridge itself ran in the
  // fake home while setting up: all background runs, all left out.
  check("Background runs are left out and counted", summary.includes("Background runs left out"));
  await caption(`${imported} items came over; background runs left out`);
  await shot("04-summary-dark");

  await caption("Codex has its own section and its own Import");
  await click("Import", '[data-import-tool="codex"] button');
  await waitFor("the Codex import to finish", `!!document.querySelector('[data-import-tool="codex"] [data-import-result="imported"]')`, 60000);
  const codexImported = Number(await js(`document.querySelector('[data-import-tool="codex"] [data-import-result="imported"]').textContent`));
  check("Codex imports from its own section", codexImported === 1, `imported=${codexImported}`);
  const claudeKept = Number((await js(`document.querySelector('[data-import-tool="claude_code"] [data-import-result="imported"]').textContent`)).replace(/,/g, ""));
  check("Claude Code's section keeps its own result", claudeKept === EXPECTED, `claude=${claudeKept}`);
  await js(`document.querySelector('[data-import-tool="codex"]').scrollIntoView({ block: "center" })`);
  await sleep(600);
  await shot("05-codex-summary-dark");

  await caption("OpenCode has its own section and its own Import");
  await click("Import", '[data-import-tool="opencode"] button');
  await waitFor("the OpenCode import to finish", `!!document.querySelector('[data-import-tool="opencode"] [data-import-result="imported"]')`, 60000);
  const openCodeImported = Number(await js(`document.querySelector('[data-import-tool="opencode"] [data-import-result="imported"]').textContent`));
  check("OpenCode imports its chat and the chat's subagent from its own section", openCodeImported === 2, `imported=${openCodeImported}`);
  check("Codex's section keeps its own result", Number(await js(`document.querySelector('[data-import-tool="codex"] [data-import-result="imported"]').textContent`)) === 1);
  check("Nothing shows OpenCode's credentials", !(await text()).includes("CANARY"));
  await js(`document.querySelector('[data-import-tool="opencode"]').scrollIntoView({ block: "center" })`);
  await sleep(600);
  await shot("05b-opencode-summary-dark");

  await caption("The OpenCode chat, with its commands, edit and subagent");
  await click("Speed up the checkout page");
  await waitFor("the OpenCode chat", `document.body.innerText.includes("0.4 s")`);
  await sleep(1200);
  const openCodeChat = await text();
  writeFileSync(join(OUT, "opencode-chat.txt"), openCodeChat);
  await shot("05c-opencode-chat-dark");
  const openCodeBadge = await js(`document.querySelector('[data-testid="import-source-badge"]')?.getAttribute("title")`);
  check("The OpenCode chat is badged with where it came from", (openCodeBadge ?? "").startsWith("Imported · OpenCode"), String(openCodeBadge));
  check("Its command, edit and delegation came over, grouped the way Bridge shows live work", openCodeChat.includes("Ran 1 command, edited 1 file, used 1 tool"), openCodeChat.slice(openCodeChat.indexOf("Imported · OpenCode"), openCodeChat.indexOf("Imported · OpenCode") + 700).replace(/\n+/g, " | "));
  check("Its thinking came over", openCodeChat.includes("Thought"));
  await click("Ran 1 command, edited 1 file", "[aria-expanded]");
  await waitFor("the OpenCode tool group to open", `[...document.querySelectorAll("[aria-expanded]")].some(el => el.textContent.includes("Ran 1 command, edited 1 file") && el.getAttribute("aria-expanded") === "true")`);
  await sleep(800);
  const openCodeTools = await text();
  check("The command, the edited file and the delegation are listed", openCodeTools.includes("npm run profile") && openCodeTools.includes("cart.ts") && openCodeTools.includes("Find every cart query"), openCodeTools.slice(openCodeTools.indexOf("Ran 1 command"), openCodeTools.indexOf("Ran 1 command") + 400).replace(/\n+/g, " | "));
  await shot("05c2-opencode-tools-open-dark");
  await caption("Its subagent is listed under it in the sidebar");
  await click("1 subagent", "[aria-label*='of Speed up the checkout page']");
  await waitFor("the subagent row", `!!document.querySelector("[data-subagent-row]")`);
  await shot("05d-opencode-subagent-row-dark");
  await click("Find every cart query", "[data-subagent-row]");
  await waitFor("the subagent's chat", `document.body.innerText.includes("Found 3 call sites: cart.ts")`);
  await sleep(1000);
  const subagentChat = await text();
  check("The imported subagent opens as history, not as a background worker", !subagentChat.includes("This is a background worker") && subagentChat.includes("Starts a new chat with this history"), subagentChat.slice(0, 400).replace(/\n+/g, " | "));
  check("The open subagent's row is marked in the sidebar", await js(`document.querySelector("[data-subagent-row][aria-current='page']")?.textContent.includes("Find every cart query") === true`));
  await shot("05e-opencode-subagent-open-dark");

  await caption("Find the imported chat in the sidebar and open it");
  await click("Invoice email header fix");
  await waitFor("the conversation to render", `document.body.innerText.includes("max-width")`);
  await sleep(1500);
  const chat = await text();
  const badge = await js(`document.querySelector('[data-testid="import-source-badge"]')?.getAttribute("title")`);
  check("The chat is badged with where it came from", (badge ?? "").startsWith("Imported · Claude Code"), String(badge));
  check("The user's screenshot came over as an image", await js(`[...document.querySelectorAll("img")].some(img => (img.alt ?? "").startsWith("Attached image"))`));
  await shot("04a-chat-before-expand-dark");
  writeFileSync(join(OUT, "chat.txt"), chat);
  check("Tool calls came over, grouped the way Bridge shows live work", chat.includes("Ran 1 command, searched code"));
  check("Thinking came over as a thinking row", chat.includes("Thought for a moment"));
  // Drawn the way Bridge draws a live compaction, at the point it happened.
  check("The compaction point came over where it happened", /Context compacted/.test(chat) && chat.indexOf("Context compacted") < chat.indexOf("Add a snapshot test"));
  await caption("Open the tool group: the calls and their output are there");
  await click("Ran 1 command, searched code", "[aria-expanded]");
  await waitFor("the tool group to open", `[...document.querySelectorAll("[aria-expanded]")].some(el => el.textContent.includes("Ran 1 command") && el.getAttribute("aria-expanded") === "true")`);
  await sleep(800);
  await shot("04b-tools-open-dark");
  const expanded = await text();
  check("The search and the failing command are listed", /logo/.test(expanded) && expanded.includes("npm run render-email"), expanded.slice(expanded.indexOf("Worked for"), expanded.indexOf("Worked for") + 300).replace(/\n+/g, " | "));
  check("The assistant's answer came over", chat.includes("max-width"));
  check("The conversation after the compaction came over", chat.includes("snapshot test"));
  await shot("04-chat-dark");
  await js(`window.scrollTo(0, 0); document.querySelector("[data-conversation-content]")?.parentElement?.scrollTo?.(0, 0)`);
  await sleep(600);

  await caption("The real Claude Code session recorded for this test");
  await click("Invoice.js test failures");
  await waitFor("the real session", `document.body.innerText.includes("discount")`);
  await sleep(1200);
  const real = await text();
  check("The real session's tool calls and answer came over", real.includes("discount") && /Ran \d+ command|Read|Explored/.test(real));
  await shot("05-real-session-dark");
  await caption("Its Explore subagent opens from the sidebar");
  await click("1 subagent", "[aria-label*='subagents of Invoice.js test failures'], [aria-label*='subagent of Invoice.js test failures']");
  await waitFor("the Explore subagent row", `[...document.querySelectorAll("[data-subagent-row]")].some(row => row.offsetParent !== null)`);
  const explore = await js(`[...document.querySelectorAll("[data-subagent-row]")].find(row => row.offsetParent !== null).getAttribute("data-subagent-row")`);
  await js(`document.querySelector(${JSON.stringify(`[data-subagent-row='${explore}']`)}).click()`);
  await waitFor("the Explore subagent's chat", `document.querySelector("[data-subagent-row][aria-current='page']") !== null`);
  await sleep(1200);
  const exploreChat = await text();
  check("The Explore subagent chat is reachable from the sidebar and opens as history", !exploreChat.includes("This is a background worker") && exploreChat.includes("A subagent's run cannot be resumed on its own"), exploreChat.slice(0, 400).replace(/\n+/g, " | "));
  await shot("05f-claude-subagent-open-dark");

  await caption("Import again: nothing lands twice");
  await click("Open settings for");
  await click("Chat history");
  await click("Import", "main button, [data-settings-column] button");
  await waitFor("the tools list", `document.body.innerText.includes("Found on this Mac") && !document.body.innerText.includes("Looking for other tools")`);
  check("The finished import is still summarised on the page", (await text()).includes("Import finished"));
  await click("Import again", "button");
  await waitFor("the second import to finish", `document.body.innerText.includes("Import finished") && document.querySelector('[data-import-result="skipped"]')?.textContent.replace(/,/g, "") === "${EXPECTED}"`, 120000);
  const importedAgain = Number(await js(`document.querySelector('[data-import-result="imported"]')?.textContent`));
  const skippedAgain = Number((await js(`document.querySelector('[data-import-result="skipped"]')?.textContent`)).replace(/,/g, ""));
  check("A second import brings over nothing new", importedAgain === 0, `imported=${importedAgain}`);
  check("Everything imported before is recognised", skippedAgain === imported, `skipped=${skippedAgain}`);
  await shot("06-again-dark");

  log("all checks passed");
} finally {
  if (RECORD) { try { await send("Page.stopScreencast"); } catch { /* closing */ } }
  writeFileSync(join(OUT, "results.json"), JSON.stringify({ results, shots }, null, 2));
  socket.close();
  chrome.kill();
}

if (RECORD && frames.length > 1) {
  const list = frames.map((frame, index) => {
    const next = frames[index + 1];
    const duration = next ? Math.max(0.04, Math.min(3, next.at - frame.at)) : 1.5;
    return `file '${frame.file}'\nduration ${duration.toFixed(3)}`;
  }).join("\n") + `\nfile '${frames.at(-1).file}'\n`;
  writeFileSync(join(OUT, "frames.txt"), list);
  const mp4 = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", join(OUT, "frames.txt"), "-vsync", "vfr", "-pix_fmt", "yuv420p", "-vf", "scale=1280:-2", join(OUT, "fake-user.mp4")]);
  log("mp4", mp4.status === 0 ? "written" : mp4.stderr.toString());
  const gif = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-i", join(OUT, "fake-user.mp4"), "-vf", "fps=8,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse", join(OUT, "fake-user.gif")]);
  log("gif", gif.status === 0 ? "written" : gif.stderr.toString());
  // The transfer card on its own, cut from the full-size frames.
  try {
    const rect = JSON.parse(readFileSync(join(OUT, "transfer-rect.json"), "utf8"));
    const even = value => Math.max(0, Math.floor(value / 2) * 2);
    // Keep the box inside the 1440×900 frame whatever the card's position.
    const x = even(rect.x), y = even(rect.y);
    const crop = `crop=${even(Math.min(rect.width, 1440 - x))}:${even(Math.min(rect.height, 900 - y))}:${x}:${y}`;
    // Eight seconds of the card from when it showed: a short loop of chats in flight.
    const start = frames[transferFrom]?.at ?? frames[0].at;
    const span = frames.slice(transferFrom).filter(frame => frame.at - start <= 8);
    const list = span.map((frame, index) => {
      const next = span[index + 1];
      return `file '${frame.file}'\nduration ${(next ? Math.max(0.02, next.at - frame.at) : 0.1).toFixed(3)}`;
    }).join("\n") + `\nfile '${span.at(-1).file}'\n`;
    writeFileSync(join(OUT, "frames-transfer.txt"), list);
    const clip = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", join(OUT, "frames-transfer.txt"), "-vsync", "vfr", "-pix_fmt", "yuv420p", "-vf", crop, join(OUT, "transfer.mp4")]);
    const clipGif = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-i", join(OUT, "transfer.mp4"), "-vf", "fps=20,scale=900:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse", join(OUT, "transfer.gif")]);
    log("transfer clip", clip.status === 0 && clipGif.status === 0 ? "written" : `${clip.stderr}${clipGif.stderr}`);
  } catch (error) { log("transfer clip skipped", String(error)); }
}
