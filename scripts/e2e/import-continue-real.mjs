#!/usr/bin/env node
// Continue an imported chat with the real Claude Code, Codex or OpenCode CLI
// (#1061, #1054, #1055; TOOL=claude_code, codex or opencode). It drives
// the real app in a browser tab attached to a real `bridged` that signs in as
// the person, imports two real sessions (made with `claude -p`, `codex exec`
// or `opencode run`), and then, like a person would:
//   1. opens the first, sees "Continues the <tool> session", asks for the
//      code word it was told, and gets it from the same native session,
//      whose transcript grows by the new turn;
//   2. deletes the second session's transcript, opens it, sees "Starts a new
//      chat with this history" and why, asks again, and gets the word from
//      the imported history.
// The imported chats must not change. Every step asserts what is on screen or
// in the store; any mismatch exits non-zero. It records screenshots and a GIF.
//
//   CLAUDE_CONFIG_DIR=<config> bridged --data-dir <dir> --listen 127.0.0.1:4391 --allowed-origin http://127.0.0.1:1471
//   LINK=... OUT=... DB=<dir>/bridge.db IMPORT_HOME=<folder holding the two sessions as projects/<slug>/<id>.jsonl>
//   SESSIONS="<id1> <word1>,<id2> <word2>" TRANSCRIPTS="<real path 1>,<real path 2>" node scripts/e2e/import-continue-real.mjs
//
// OpenCode keeps sessions as rows, not files: IMPORT_HOME is its data folder
// and both TRANSCRIPTS are its store (`opencode.db`). A session's length is its
// rows, and "deleting its transcript" deletes its rows, so run it against a
// scratch data folder (XDG_DATA_HOME), never your own.

import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const LINK = process.env.LINK;
const OUT = process.env.OUT ?? "/tmp/import-continue";
const PORT = Number(process.env.CDP_PORT ?? 9377);
const RECORD = process.env.RECORD !== "0";
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

const TOOL = process.env.TOOL ?? "claude_code";
const LABEL = { claude_code: "Claude Code", codex: "Codex", opencode: "OpenCode" }[TOOL];
const NATIVE = { claude_code: "claude", codex: "codex", opencode: "opencode" }[TOOL];
if (!LABEL) throw new Error(`TOOL must be claude_code, codex or opencode, not ${TOOL}`);
const ROWS = TOOL === "opencode";
// The second session's copy in IMPORT_HOME, removed with the real one.
const SECOND_LINK = process.env.SECOND_LINK;
const DB = process.env.DB;
const IMPORT_HOME = process.env.IMPORT_HOME;
const [first, second] = (process.env.SESSIONS ?? "").split(",").map(item => { const [id, word] = item.trim().split(/\s+/); return { id, word }; });
const [firstTranscript, secondTranscript] = (process.env.TRANSCRIPTS ?? "").split(",");
if (!DB || !IMPORT_HOME || !first?.word || !second?.word || !secondTranscript) throw new Error("Set DB, IMPORT_HOME, SESSIONS and TRANSCRIPTS");
const sql = query => {
  const run = spawnSync("sqlite3", ["-json", DB, query], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(`sqlite3: ${run.stderr}`);
  return run.stdout.trim() ? JSON.parse(run.stdout) : [];
};
const storeSql = (store, query) => {
  const run = spawnSync("sqlite3", [store, query], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(`sqlite3 ${store}: ${run.stderr}`);
  return run.stdout.trim();
};
/** How long a session is: its transcript's lines, or OpenCode's rows. */
const length = (path, session) => ROWS
  ? Number(storeSql(path, `SELECT COUNT(*) FROM session_message WHERE session_id='${session}'`))
  : readFileSync(path, "utf8").split("\n").filter(Boolean).length;
/** The tool forgets a session: its transcript goes, or OpenCode's rows do. */
function forget(path, session) {
  if (ROWS) {
    storeSql(path, `DELETE FROM session_message WHERE session_id='${session}'; DELETE FROM session_v2 WHERE id='${session}';`);
    return;
  }
  rmSync(path, { force: true });
  rmSync(SECOND_LINK ?? join(IMPORT_HOME, "projects", path.split("/").at(-2), path.split("/").at(-1)), { force: true });
}
const entries = id => JSON.stringify(sql(`SELECT id, kind, payload, created_at FROM session_entries WHERE session_id='${id}' ORDER BY sequence`));

const QUESTION = "What code word did I ask you to remember earlier in this conversation? Answer from the conversation alone, without running any tools. Reply with just the word, in lowercase.";
async function ask(question) {
  await js(`document.querySelector('textarea[aria-label="Message Bridge"]').focus()`);
  await send("Input.insertText", { text: question });
  await sleep(400);
  await click("Send", "button[aria-label='Send']");
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
  if (await js(`document.body.innerText.includes("Choose agents")`)) {
    const done = await js(`(async () => {
      const { bridgeApi } = await import("/src/api.ts");
      const { recommendedProfileDrafts } = await import("/src/modelProfiles.ts");
      const health = await bridgeApi.health();
      const setup = await bridgeApi.saveModelProfiles(recommendedProfileDrafts(health.adapters.filter(a => a.id === ${JSON.stringify(NATIVE)}), { allowAvailableFallback: true }));
      return setup.complete;
    })()`);
    check(`Setup completes for someone who uses ${LABEL}`, done === true);
  }

  // The two real sessions, imported the way a folder import does.
  const imported = await js(`(async () => {
    const { bridgeApi } = await import("/src/api.ts");
    const discovery = await bridgeApi.discoverExternalImport({ provider: ${JSON.stringify(TOOL)}, approvedRoots: [${JSON.stringify(IMPORT_HOME)}], selectedExport: null, sourceVersion: null, schemaVersion: null, formatVersions: {} });
    const ids = discovery.artifacts.filter(a => a.kind === "conversation").map(a => a.artifactId);
    const preview = await bridgeApi.previewExternalImport(discovery, ids);
    const chosen = preview.candidates.filter(c => c.kind === "conversation").map(c => c.candidateId);
    const commit = await bridgeApi.commitExternalImport(discovery.discoveryId, preview.candidates, { selectedCandidateIds: chosen, conflictPolicy: "skip", setupActivationPolicy: "disabled", memoryScope: null, dryRun: false, createdAt: new Date().toISOString() }, ids);
    return commit.imported;
  })()`);
  check(`Both real ${LABEL} sessions are imported`, imported === 2, `imported=${imported}`);
  const importedId = sid => sql(`SELECT session_id FROM import_native_sessions WHERE native_key='${NATIVE}:${sid}'`)[0]?.session_id;
  const firstImported = importedId(first.id);
  const secondImported = importedId(second.id);
  check(`Bridge recorded each ${LABEL} session id`, !!firstImported && !!secondImported);
  const before = { first: entries(firstImported), second: entries(secondImported) };
  await send("Page.navigate", { url: LINK });
  await waitFor("the app to load", `!!document.querySelector('button[aria-label^="Open settings for"]')`, 30000);
  await sleep(1500);

  await caption(`An imported ${LABEL} chat: ${LABEL} still has the session`);
  await click(first.word, "[data-session-id], button, a, [role=button]");
  await waitFor("the continuation notice", `!!document.querySelector("[data-import-continuation]")`, 15000);
  const nativeNotice = await js(`document.querySelector("[data-import-continuation]").innerText`);
  check(`The composer says the next message continues the ${LABEL} session`, nativeNotice.includes(`Continues the ${LABEL} session`) && /stay behind/.test(nativeNotice), nativeNotice.replace(/\n+/g, " | "));
  check("It says so before anything is sent", await js(`document.querySelector("[data-import-continuation]").dataset.importContinuation === "native"`));
  await shot("01-native-notice-dark");
  const linesBefore = length(firstTranscript, first.id);

  await caption("Ask it what it was told to remember");
  await ask(QUESTION);
  // Lowercase: the copied history shows the word only in capitals, so only a
  // real answer can match.
  await waitFor(`the reply from the same ${LABEL} session`, `document.body.innerText.includes(${JSON.stringify(first.word.toLowerCase())})`, 180000);
  await sleep(2500);
  const firstContinuation = sql(`SELECT id, kind, provider_session_id FROM sessions WHERE fork_parent_session_id='${firstImported}'`);
  check("A new chat continues it", firstContinuation.length === 1 && firstContinuation[0].kind === "direct", JSON.stringify(firstContinuation));
  check(`It resumed the same ${LABEL} session`, firstContinuation[0]?.provider_session_id === first.id, firstContinuation[0]?.provider_session_id);
  const linesAfter = length(firstTranscript, first.id);
  check(`${LABEL}'s own transcript gained the new turn`, linesAfter > linesBefore, `${linesBefore} -> ${linesAfter}`);
  check("The answer came from that session's memory", (await text()).includes(first.word.toLowerCase()));
  await shot("02-native-reply-dark");

  await caption(`${LABEL} no longer has the second session`);
  forget(secondTranscript, second.id);
  await click(second.word, "[data-session-id], button, a, [role=button]");
  await waitFor("the continuation notice", `document.querySelector("[data-import-continuation]")?.dataset.importContinuation === "history"`, 15000);
  const historyNotice = await js(`document.querySelector("[data-import-continuation]").innerText`);
  check("The composer says it starts a new chat with this history, and why", /Starts a new chat with this history/.test(historyNotice) && historyNotice.includes(`${LABEL} no longer has this session`), historyNotice.replace(/\n+/g, " | "));
  await shot("03-history-notice-dark");

  await caption("Ask again: the history carries the word");
  await ask(QUESTION);
  await waitFor("the reply from the history", `document.body.innerText.includes(${JSON.stringify(second.word.toLowerCase())})`, 180000);
  await sleep(2500);
  const secondContinuation = sql(`SELECT id, kind, provider_session_id FROM sessions WHERE fork_parent_session_id='${secondImported}'`);
  check("A new chat continues it from the history", secondContinuation.length === 1 && secondContinuation[0].kind === "direct");
  check("The answer came from the imported history", (await text()).includes(second.word.toLowerCase()));
  check(`It did not claim the gone ${LABEL} session`, secondContinuation[0]?.provider_session_id !== second.id, String(secondContinuation[0]?.provider_session_id));
  await shot("04-history-reply-dark");

  check("Both imported chats are unchanged, entry for entry", entries(firstImported) === before.first && entries(secondImported) === before.second);
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
}
