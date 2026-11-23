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
// How many generated chats the fake home holds (import-fake-home.py), plus 5
// hand-made in ~/.claude, 1 that ran in a Conductor workspace, 1 that T3 Code
// ran in its worktree, 1 in a subfolder of the storefront repository, 1 in
// the work account's folder, and 1 /usage probe. The probe is counted when
// found (nothing is read then) and left out when imported.
const CHATS = Number(process.env.IMPORT_FAKE_CHATS ?? 600) + 10;
const PROBES = 1;
// The fake home, to check on disk that Bridge left Conductor's, T3 Code's and
// Orca's folders alone.
const FAKE_HOME = process.env.FAKE_HOME;
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
  // The fake user also has a work Claude account, added in Settings with its
  // own folder.
  const accountSaved = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const state = await bridgeApi.configState();
    const claude = state.harnesses.find(item => item.id === "claude");
    const saved = await bridgeApi.saveHarnessConfig({ ...claude, advanced: { ...(claude.advanced ?? {}), accounts: [{ id: "work", label: "Work", homePath: "~/.claude-work" }] } });
    return saved.harnesses.find(item => item.id === "claude").advanced.accounts.length;
  })()`);
  check("A work Claude account with its own folder is in Settings", accountSaved === 1, String(accountSaved));
  await caption("Open Settings, then Import");
  await click("Open settings for");
  await waitFor("Settings to open", `[...document.querySelectorAll("button, a, [role=button]")].some(el => el.offsetParent !== null && el.textContent.trim() === "Archived chats")`);
  const sections = await js(`[...document.querySelectorAll("button, a, [role=button]")].filter(el => el.offsetParent !== null).map(el => el.textContent.trim())`);
  check("Import has its own place in Settings, beside Archived chats", sections.includes("Import") && sections.includes("Archived chats"), JSON.stringify(sections).slice(0, 300));
  await shot("00-settings-dark");
  await click("Import");
  await waitFor("the tools list", `document.body.innerText.includes("Found on this Mac") && !document.body.innerText.includes("Looking for other tools")`);
  await caption("Bridge finds the tools already on this Mac, without reading any chat");
  const found = await text();
  check("Claude Code is found with its chats from both folders, projects, setup and the background run left out", new RegExp(`${CHATS.toLocaleString("en-US")} chats in 9 projects · from 2 folders · 1 setup item · \\d+ background runs? left out`).test(found), found.slice(found.indexOf("Found on this Mac"), found.indexOf("Found on this Mac") + 700).replace(/\n+/g, " | "));
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
  const EXPECTED = CHATS + 2 - PROBES;
  await caption("One click: Import");
  await click("Import", '[data-import-tool="claude_code"] button');
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
  await click("Storage");
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
  const placed = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const state = await bridgeApi.state();
    const badge = state.sessions.find(item => (item.title ?? "").startsWith("Make the cart badge show the item count"));
    const workspace = state.workspaces.find(item => item.id === badge?.workspaceId);
    return {
      project: workspace?.path ?? null,
      title: workspace?.title ?? null,
      probe: state.sessions.some(item => (item.title ?? "").startsWith("/usage")),
      bridgeRun: state.sessions.some(item => (item.title ?? "").includes("Slack inbox")),
    };
  })()`);
  check("A chat from a repository subfolder lands under the repository's project, made for it", (placed.project ?? "").endsWith("/code/storefront") && placed.title === "storefront", JSON.stringify(placed));
  check("A /usage probe and a run Bridge made itself did not come back as chats", !placed.probe && !placed.bridgeRun, JSON.stringify(placed));
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
  await click("Back to app");
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

  await caption("The work account's chat came over too");
  await click("Rotate the Stripe keys");
  await waitFor("the work account's chat", `document.body.innerText.includes("Rotated both keys")`);
  await sleep(1000);
  check("The chat from the work account's folder is in the sidebar and opens", (await text()).includes("Rotated both keys and updated the deploy secrets."));
  await shot("05g-account-chat-dark");

  await caption("Conductor's workspaces come over where they are");
  await click("Open settings for");
  await waitFor("Settings to open", `[...document.querySelectorAll("button, a, [role=button]")].some(el => el.offsetParent !== null && el.textContent.trim() === "Archived chats")`);
  await click("Import");
  await waitFor("the tools list", `!!document.querySelector('[data-import-tool="conductor"]') && !document.body.innerText.includes("Looking for other tools")`);
  const conductorFound = await js(`document.querySelector('[data-import-tool="conductor"]').innerText`);
  check("Conductor is found with its workspaces and its own Import button", conductorFound.includes("2 workspaces in 1 repo") && conductorFound.includes("left where they are")
    && await js(`!!document.querySelector('[data-import-tool="conductor"] button')`), conductorFound.replace(/\n+/g, " | "));
  await click("Import", '[data-import-tool="conductor"] button');
  await waitFor("the Conductor import to finish", `!!document.querySelector('[data-import-tool="conductor"] [data-import-result="imported"]')`, 60000);
  const conductorSummary = await js(`document.querySelector('[data-import-tool="conductor"]').innerText`);
  check("Conductor's active workspace came over, the archived one was left out, and the detached one needs a branch",
    Number(await js(`document.querySelector('[data-import-tool="conductor"] [data-import-result="imported"]').textContent`)) === 1
      && conductorSummary.includes("Archived in Conductor, left out") && conductorSummary.includes("need a branch first"),
    conductorSummary.replace(/\n+/g, " | "));
  check("Nothing shows Conductor's secrets", !(await text()).includes("CANARY"));
  await js(`document.querySelector('[data-import-tool="conductor"]').scrollIntoView({ block: "center" })`);
  await sleep(600);
  await shot("05h-conductor-summary-dark");
  const adopted = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const state = await bridgeApi.state();
    const workspace = state.workspaces.find(item => item.title === "Fix the cart total");
    const chat = state.sessions.find(item => item.title === "Cart total ignores quantities");
    return { id: workspace?.id, path: workspace?.path, branch: workspace?.branch, chatWorkspace: chat?.workspaceId };
  })()`);
  check("The workspace keeps Conductor's name, folder and branch", !!adopted.id && adopted.path.endsWith("/conductor/workspaces/storefront/tokyo") && adopted.branch === "fix-cart-total", JSON.stringify(adopted));
  check("The Claude Code chat that ran there is under it", adopted.chatWorkspace === adopted.id, JSON.stringify(adopted));
  await caption("Archiving it in Bridge leaves Conductor's folder alone");
  const archived = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    await bridgeApi.archiveWorkspace(${JSON.stringify(adopted.id)});
    const state = await bridgeApi.state();
    return !state.workspaces.some(item => item.id === ${JSON.stringify(adopted.id)});
  })()`);
  const { existsSync, readFileSync: read } = await import("node:fs");
  const kept = FAKE_HOME ? existsSync(join(FAKE_HOME, "conductor/workspaces/storefront/tokyo/cart.js")) && read(join(FAKE_HOME, "conductor/workspaces/storefront/tokyo/cart.js"), "utf8").includes("uncommitted") : null;
  check("Archived in Bridge, the folder and its uncommitted work are still there for Conductor", archived === true && kept !== false, `archived=${archived} kept=${kept}`);

  await caption("T3 Code's threads come over as chats, its worktree as a workspace");
  await js(`document.querySelector('[data-import-tool="t3code"]').scrollIntoView({ block: "center" })`);
  const t3Found = await js(`document.querySelector('[data-import-tool="t3code"]').innerText`);
  check("T3 Code is found with its chats and worktree, and its own Import button", t3Found.includes("2 chats · 1 workspace in 1 repo")
    && await js(`!!document.querySelector('[data-import-tool="t3code"] button')`), t3Found.replace(/\n+/g, " | "));
  await click("Import", '[data-import-tool="t3code"] button');
  await waitFor("the T3 Code import to finish", `!!document.querySelector('[data-import-tool="t3code"] [data-import-result="imported"]')`, 60000);
  const t3 = await js(`(() => {
    const section = document.querySelector('[data-import-tool="t3code"]');
    const count = selector => Number(section.querySelector(selector)?.textContent ?? -1);
    return { imported: count('[data-import-result="imported"]'), skipped: count('[data-import-result="skipped"]'), workspaces: count('[data-import-result="workspaces"]'), text: section.innerText };
  })()`);
  check("The thread whose Claude Code session is gone came over from T3 Code's copy; the one Claude Code still has was already in Bridge",
    t3.imported === 1 && t3.skipped === 1, JSON.stringify(t3).slice(0, 400));
  check("Its worktree came over as a workspace, left where it is", t3.workspaces === 1 && t3.text.includes("so T3 Code keeps working with them"), t3.text.replace(/\n+/g, " | "));
  check("Nothing shows T3 Code's secrets", !(await text()).includes("CANARY"));
  await sleep(600);
  await shot("05i-t3code-summary-dark");
  const t3State = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const state = await bridgeApi.state();
    const workspace = state.workspaces.find(item => item.title === "Add a discount helper");
    const chat = state.sessions.find(item => item.title === "Add a discount helper");
    return {
      id: workspace?.id, path: workspace?.path, branch: workspace?.branch, chatWorkspace: chat?.workspaceId,
      copy: state.sessions.some(item => item.title === "Casual conversation"),
      deleted: state.sessions.some(item => item.title === "Thrown away"),
    };
  })()`);
  check("The workspace keeps T3 Code's folder and branch, and the chat that ran there is under it",
    !!t3State.id && (t3State.path ?? "").endsWith("/.t3/worktrees/pricing/feat-discount-07db51fef00c511d") && t3State.branch === "feat/discount" && t3State.chatWorkspace === t3State.id, JSON.stringify(t3State));
  check("The thread deleted in T3 Code did not come over", t3State.copy && !t3State.deleted, JSON.stringify(t3State));
  await caption("The chat from T3 Code's own copy opens");
  await click("Back to app");
  await click("Casual conversation");
  await waitFor("the T3 Code chat", `document.body.innerText.includes("What are we working on today?")`);
  await sleep(1000);
  const t3Badge = await js(`document.querySelector('[data-testid="import-source-badge"]')?.getAttribute("title")`);
  check("It is badged as imported from T3 Code", (t3Badge ?? "").startsWith("Imported · T3 Code"), String(t3Badge));
  // The finished-import notice sits over the top of the chat; close it to see both sides.
  await js(`document.querySelectorAll('button[aria-label="Dismiss notification"]').forEach(button => button.click())`);
  await sleep(800);
  const t3Chat = await text();
  const t3Said = t3Chat.slice(0, t3Chat.indexOf("What are we working on today?"));
  check("Both sides of the conversation came over, the user's message first", /(^|\n)hi\n/.test(t3Said), t3Said.slice(-200).replace(/\n+/g, " | "));
  await shot("05j-t3code-chat-dark");
  const t3Tree = FAKE_HOME ? join(FAKE_HOME, ".t3/worktrees/pricing/feat-discount-07db51fef00c511d/price.js") : null;
  const t3Kept = t3Tree ? existsSync(t3Tree) && read(t3Tree, "utf8").includes("uncommitted") : null;
  check("T3 Code's worktree and its uncommitted work are untouched", t3Kept !== false, `kept=${t3Kept}`);

  await caption("Orca's chats, worktree and multi-agent run come over");
  await click("Open settings for");
  await waitFor("Settings to open", `[...document.querySelectorAll("button, a, [role=button]")].some(el => el.offsetParent !== null && el.textContent.trim() === "Archived chats")`);
  await click("Import");
  await waitFor("the tools list", `!!document.querySelector('[data-import-tool="orca"]') && !document.body.innerText.includes("Looking for other tools")`);
  await js(`document.querySelector('[data-import-tool="orca"]').scrollIntoView({ block: "center" })`);
  const orcaFound = await js(`document.querySelector('[data-import-tool="orca"]').innerText`);
  check("Orca is found with its chats and worktree, and its own Import button", orcaFound.includes("2 chats · 1 workspace in 1 repo")
    && await js(`!!document.querySelector('[data-import-tool="orca"] button')`), orcaFound.replace(/\n+/g, " | "));
  await click("Import", '[data-import-tool="orca"] button');
  await waitFor("the Orca import to finish", `!!document.querySelector('[data-import-tool="orca"] [data-import-result="imported"]')`, 60000);
  const orcaResult = await js(`(() => {
    const section = document.querySelector('[data-import-tool="orca"]');
    const count = selector => Number(section.querySelector(selector)?.textContent ?? -1);
    return { imported: count('[data-import-result="imported"]'), skipped: count('[data-import-result="skipped"]'), workspaces: count('[data-import-result="workspaces"]'), text: section.innerText };
  })()`);
  check("The worker came over from Orca's own copy; the lead chat OpenCode already brought was already in Bridge",
    orcaResult.imported === 1 && orcaResult.skipped === 1, JSON.stringify(orcaResult).slice(0, 400));
  check("Its worktree came over as a workspace, left where it is", orcaResult.workspaces === 1 && orcaResult.text.includes("so Orca keeps working with them"), orcaResult.text.replace(/\n+/g, " | "));
  check("What did not come over says why: the Pi chat and the worker Orca kept no session for",
    orcaResult.text.includes("chats with pi, which Bridge does not run") && orcaResult.text.includes("workers' place under their run's lead chat"), orcaResult.text.replace(/\n+/g, " | "));
  check("Nothing shows Orca's secrets", !(await text()).includes("CANARY"));
  await sleep(600);
  await shot("05k-orca-summary-dark");
  const orcaState = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const state = await bridgeApi.state();
    const workspace = state.workspaces.find(item => item.title === "Refund email copy");
    const lead = state.sessions.find(item => item.title === "Speed up the checkout page");
    const worker = state.sessions.find(item => item.title === "Write the refund email copy");
    return {
      id: workspace?.id, path: workspace?.path, branch: workspace?.branch,
      workerWorkspace: worker?.workspaceId, workerParent: worker?.parentSessionId, lead: lead?.id,
      leftOut: state.sessions.some(item => item.title === "Ask Pi about fonts" || item.title === "Thrown away in Orca"),
    };
  })()`);
  check("The workspace keeps Orca's name, folder and branch, and the worker's chat is in it",
    !!orcaState.id && (orcaState.path ?? "").endsWith("/orca/workspaces/storefront/refund-copy") && orcaState.branch === "refund-copy" && orcaState.workerWorkspace === orcaState.id, JSON.stringify(orcaState));
  check("The worker is under the chat that led its run", !!orcaState.lead && orcaState.workerParent === orcaState.lead, JSON.stringify(orcaState));
  check("The Pi chat and the chat deleted in Orca did not come over", !orcaState.leftOut, JSON.stringify(orcaState));
  await caption("The worker's chat, from Orca's own copy, opens");
  await click("Back to app");
  await click("Write the refund email copy");
  await waitFor("the Orca chat", `document.body.innerText.includes("Your refund is on its way")`);
  await sleep(1000);
  const orcaBadge = await js(`document.querySelector('[data-testid="import-source-badge"]')?.getAttribute("title")`);
  check("It is badged as imported from Orca", (orcaBadge ?? "").startsWith("Imported · Orca"), String(orcaBadge));
  await js(`document.querySelectorAll('button[aria-label="Dismiss notification"]').forEach(button => button.click())`);
  await sleep(800);
  const orcaChat = await text();
  const orcaSaid = orcaChat.slice(0, orcaChat.indexOf("Your refund is on its way"));
  check("Both sides of the conversation came over, the user's message first", orcaSaid.includes("Write the copy for the refund confirmation email."), orcaSaid.slice(-200).replace(/\n+/g, " | "));
  await shot("05l-orca-chat-dark");
  const orcaTree = FAKE_HOME ? join(FAKE_HOME, "orca/workspaces/storefront/refund-copy/cart.js") : null;
  const orcaKept = orcaTree ? existsSync(orcaTree) && read(orcaTree, "utf8").includes("uncommitted") : null;
  check("Orca's worktree and its uncommitted work are untouched", orcaKept !== false, `kept=${orcaKept}`);

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
  await waitFor("the Explore subagent row", `[...document.querySelectorAll("[data-subagent-row]")].some(row => row.offsetParent !== null && row.textContent.includes("Explore:"))`);
  await click("Explore:", "[data-subagent-row]");
  await waitFor("the Explore subagent's chat", `document.querySelector("[data-subagent-row][aria-current='page']")?.textContent.includes("Explore:") === true && document.querySelector('[data-testid="import-source-badge"]')?.getAttribute("title")?.startsWith("Imported · Claude Code") === true`);
  await sleep(1200);
  const exploreChat = await text();
  check("The Explore subagent chat is reachable from the sidebar and opens as history", !exploreChat.includes("This is a background worker") && exploreChat.includes("A subagent's run cannot be resumed on its own") && !exploreChat.includes("Found 3 call sites"), exploreChat.slice(0, 400).replace(/\n+/g, " | "));
  await shot("05f-claude-subagent-open-dark");

  await caption("Import again: nothing lands twice");
  await click("Open settings for");
  await waitFor("Settings to open", `[...document.querySelectorAll("button, a, [role=button]")].some(el => el.offsetParent !== null && el.textContent.trim() === "Archived chats")`);
  await click("Import");
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
