#!/usr/bin/env node
// Run the personal-app evals against real agents and real MCP servers.
//
//   node scripts/eval-personal-apps.mjs [--phases build,render,lock,routing]
//        [--ids docs-codex,...] [--chats codex,claude] [--data-dir <dir>]
//        [--bin-dir <dir with bridge and bridged>] [--concurrency 2] [--port 1491]
//        [--opencode-model opencode-go/glm-5.3] [--codex-model ...] [--claude-model ...]
//
// A `bridged` daemon runs on a fresh data directory, so nothing touches the
// person's own Bridge state. Each build case is one real chat (`bridge exec`)
// that should call `apps_draft`; Bridge's stored draft is then scored: a live
// read with the draft's sample arguments, the view rendered in Bridge's own
// McpAppFrame in light and dark with every network attempt recorded, and a
// lock. The routing phase sends fixed asks to fresh chats and records which
// app (if any) `apps_open` opened. Writes are never approved: a view's write
// returns Bridge's "would send" preview. Results land in
// testing/evals/personal-apps/results/<run>/.
import { execFile, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { DatabaseSync } from "node:sqlite";
import { BUILD_CASES, ROUTING_ASKS } from "./eval-personal-apps/cases.mjs";
import { buildOutcome, classifyRouting, isToolError, markdownReport, routingSummary, scoreBuild } from "./eval-personal-apps/score.mjs";

const run = promisify(execFile);
/** The servers the apps are for: only calls to these count as a chat
 *  answering with the server's own tools instead of opening its app. */
const APP_SERVERS = new Set(BUILD_CASES.map(item => item.server));
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const phases = new Set(flag("phases", "build,render,lock,routing").split(","));
const ids = flag("ids", undefined)?.split(",");
const chats = flag("chats", "codex,claude").split(",");
const concurrency = Number(flag("concurrency", "2"));
// A model per agent, where the account's default would bill elsewhere: an
// OpenCode Go plan covers only the opencode-go provider.
const models = { opencode: flag("opencode-model", "opencode-go/glm-5.3"), codex: flag("codex-model", undefined), claude: flag("claude-model", undefined) };
const binDir = flag("bin-dir", process.env.BRIDGE_EVAL_BIN_DIR ?? join(root, "src-tauri/target/debug"));
const BRIDGE = join(binDir, "bridge");
const BRIDGED = join(binDir, "bridged");
for (const binary of [BRIDGE, BRIDGED]) {
  if (!existsSync(binary)) throw new Error(`build it first: cargo build -p bridge-client --bin bridge -p bridged --bin bridged (looked for ${binary})`);
}
// A short base: the daemon's socket lives in the data directory, and a Unix
// socket path is limited to about 104 bytes (macOS's tmpdir is long).
const shortTmp = existsSync("/private/tmp") ? "/private/tmp" : tmpdir();
// `--resume` finishes an earlier run in place, on that run's own data.
const resumeDir = flag("resume", undefined);
const resumed = resumeDir ? JSON.parse((await import("node:fs")).readFileSync(join(resumeDir, "results.json"), "utf8")) : null;
const dataDir = resumed?.dataDir ?? flag("data-dir", undefined) ?? mkdtempSync(join(shortTmp, "app-evals-"));
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const out = resumeDir ?? join(root, "testing/evals/personal-apps/results", stamp);
for (const sub of ["build", "routing", "screenshots"]) mkdirSync(join(out, sub), { recursive: true });
const log = (...parts) => console.error(`[eval ${new Date().toISOString().slice(11, 19)}]`, ...parts);

// --- the daemon --------------------------------------------------------------

async function call(method, params) {
  const argv = ["exec", "--json", "--data-dir", dataDir, "--timeout", "300", "--method", method];
  if (params !== undefined) argv.push("--params", JSON.stringify(params));
  const { stdout } = await run(BRIDGE, argv, { maxBuffer: 64 * 1024 * 1024 }).catch(error => ({ stdout: error.stdout ?? "" }));
  const line = stdout.trim().split("\n").filter(Boolean).map(text => JSON.parse(text)).find(item => item.method === method);
  if (!line) throw new Error(`${method}: no answer`);
  if (line.type === "error") throw new Error(line.message);
  return line.result;
}

async function startDaemon() {
  const daemon = spawn(BRIDGED, ["--data-dir", dataDir, "--health-addr", "none"], { stdio: ["ignore", "ignore", "pipe"] });
  daemon.stderr.on("data", chunk => { if (/panic|error/i.test(chunk)) log("daemon:", String(chunk).trim().slice(0, 300)); });
  // Up, then every agent a case needs discovered: OpenCode's discovery
  // finishes seconds after the daemon answers.
  const needed = new Set([...BUILD_CASES.map(item => item.chat), ...chats]);
  let last = null;
  for (let i = 0; i < 120; i++) {
    try {
      last = (await call("health/health")).harnesses;
      if ([...needed].every(harness => last?.[harness])) return daemon;
    } catch {
      // Not up yet.
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  if (last) {
    log("agents still unavailable after 60 s:", JSON.stringify(last));
    return daemon;
  }
  throw new Error("bridged did not come up");
}

/** One real chat turn; returns its JSONL events. */
async function turn(harness, prompt, file) {
  return new Promise(resolve => {
    const argv = ["exec", "--json", "--data-dir", dataDir, "--timeout", "900", "--harness", harness];
    if (models[harness]) argv.push("--model", models[harness]);
    const child = spawn(BRIDGE, [...argv, prompt], { stdio: ["ignore", "pipe", "pipe"] });
    let text = "";
    child.stdout.on("data", chunk => { text += chunk; });
    child.on("close", () => {
      writeFileSync(file, text);
      resolve(text.trim().split("\n").filter(Boolean).flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } }));
    });
  });
}

async function pool(items, worker) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.max(1, concurrency) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await worker(items[index], index);
    }
  }));
  return results;
}

/** The draft a chat made, from Bridge's own record, for chats whose
 *  events hide the call: OpenCode reaches MCP tools only through its code
 *  mode (`execute`), so `apps_draft` never shows as a tool of its own. */
function draftOfSession(sessionId, dir = dataDir) {
  if (!sessionId) return null;
  const db = new DatabaseSync(join(dir, "bridge.db"), { readOnly: true });
  try {
    const row = db.prepare("SELECT draft_id, app_id, content_hash FROM personal_app_draft_sessions WHERE session_id=? ORDER BY created_at DESC LIMIT 1").get(sessionId);
    return row ? { draftId: row.draft_id, appId: row.app_id, contentHash: row.content_hash, fromRecord: true } : null;
  } finally {
    db.close();
  }
}

function withRecordedDraft(outcome, dir) {
  if (outcome.drafted) return outcome;
  const draft = draftOfSession(outcome.sessionId, dir);
  return draft ? { ...outcome, drafted: true, draft, attempts: Math.max(outcome.attempts, 1) } : outcome;
}

/** A draft's manifest; once locked the draft row is gone and the locked
 *  version with the same content hash holds it. */
function manifestOf(draftId, contentHash) {
  const db = new DatabaseSync(join(dataDir, "bridge.db"), { readOnly: true });
  try {
    const row = db.prepare("SELECT manifest FROM personal_app_drafts WHERE id=?").get(draftId)
      ?? (contentHash ? db.prepare("SELECT manifest FROM personal_app_versions WHERE content_hash=? ORDER BY version DESC LIMIT 1").get(contentHash) : undefined);
    return row ? JSON.parse(row.manifest) : null;
  } finally {
    db.close();
  }
}

/** The app a content hash was locked as, if it was. */
function lockedAs(contentHash) {
  const db = new DatabaseSync(join(dataDir, "bridge.db"), { readOnly: true });
  try {
    return db.prepare("SELECT app_id FROM personal_app_versions WHERE content_hash=? ORDER BY version DESC LIMIT 1").get(contentHash)?.app_id ?? null;
  } finally {
    db.close();
  }
}

/** For each case, every locked app for the same server: opening any of
 *  them answers an ask about that server. */
function lockedAppsByCase(entries) {
  const byServer = new Map();
  for (const entry of entries.filter(item => item.lock?.ok)) {
    byServer.set(entry.testCase.server, [...(byServer.get(entry.testCase.server) ?? []), entry.lock.appId]);
  }
  return new Map(BUILD_CASES.map(item => [item.id, byServer.get(item.server) ?? []]));
}

// --- phases ------------------------------------------------------------------

async function preview(entry, tool, argumentsValue) {
  const started = Date.now();
  try {
    const result = await call("apps/preview_personal_app_tool", { sessionId: entry.sessionId, draftId: entry.draftId, contentHash: entry.contentHash, tool, arguments: argumentsValue });
    return { tool, ok: true, isError: isToolError(result), ms: Date.now() - started, result };
  } catch (error) {
    return { tool, ok: false, error: String(error.message ?? error), ms: Date.now() - started };
  }
}

async function build(testCase) {
  log(`build ${testCase.id} (${testCase.chat})`);
  const started = Date.now();
  const events = await turn(testCase.chat, testCase.prompt, join(out, "build", `${testCase.id}.jsonl`));
  const outcome = withRecordedDraft(buildOutcome(events));
  const entry = { testCase, outcome, ms: Date.now() - started };
  if (outcome.drafted) {
    entry.draftId = outcome.draft.draftId;
    entry.contentHash = outcome.draft.contentHash;
    entry.sessionId = outcome.sessionId;
    entry.manifest = manifestOf(entry.draftId, entry.contentHash);
    // The sample arguments are the view's input, not any one tool's, so
    // each read is tried until one answers without a server error.
    for (const read of entry.manifest?.reads ?? []) {
      entry.liveRead = await preview(entry, read, entry.manifest.sampleArgs ?? {});
      if (entry.liveRead.ok && !entry.liveRead.isError) break;
    }
  }
  log(`build ${testCase.id}: ${outcome.drafted ? `drafted ${outcome.draft.appId}` : "no draft"} in ${Math.round(entry.ms / 1000)} s`);
  return entry;
}

async function render(entries) {
  const drafted = entries.filter(entry => entry.manifest);
  if (!drafted.length) return;
  const { createServer } = await import("vite");
  const byId = new Map(drafted.map(entry => [entry.testCase.id, entry]));
  const api = {
    name: "personal-app-eval-api",
    configureServer(server) {
      server.middlewares.use(async (request, response, next) => {
        if (!request.url?.startsWith("/eval-api/")) return next();
        const url = new URL(request.url, "http://local");
        const send = value => { response.setHeader("content-type", "application/json"); response.end(JSON.stringify(value)); };
        if (url.pathname === "/eval-api/view") {
          const entry = byId.get(url.searchParams.get("id"));
          if (!entry) return send({ error: "unknown" });
          return send({ name: entry.manifest.name, html: entry.manifest.viewHtml, resourceUri: `ui://bridge/personal/${entry.outcome.draft.appId}@${entry.contentHash}`, input: entry.manifest.sampleArgs ?? {} });
        }
        if (url.pathname === "/eval-api/call") {
          let body = "";
          for await (const chunk of request) body += chunk;
          const { id, tool, arguments: toolArguments } = JSON.parse(body);
          const entry = byId.get(id);
          const result = await preview(entry, tool, toolArguments ?? {});
          (entry.viewCalls ??= []).push({ tool, arguments: toolArguments, ok: result.ok, isError: result.isError ?? false, ms: result.ms, error: result.error ?? null, preview: entry.manifest.writes.includes(tool) });
          return send(result.ok ? { result: result.result } : { error: result.error });
        }
        next();
      });
    },
  };
  const port = Number(flag("port", "1491"));
  const server = await createServer({ root, configFile: join(root, "vite.config.ts"), server: { port, strictPort: true }, plugins: [api], logLevel: "error" });
  await server.listen();
  const { chromium } = await import(process.env.BRIDGE_PLAYWRIGHT_MODULE ?? "playwright");
  const browser = await chromium.launch();
  try {
    for (const entry of drafted) {
      entry.render = {};
      for (const theme of ["light", "dark"]) {
        const context = await browser.newContext({ viewport: { width: 900, height: 900 }, colorScheme: theme });
        const page = await context.newPage();
        const networkAttempts = [];
        const cspViolations = [];
        page.on("console", message => { if (/Content Security Policy|Refused to/i.test(message.text())) cspViolations.push(message.text().slice(0, 200)); });
        page.on("request", request => {
          const fromView = request.frame()?.url().startsWith("about:srcdoc") || request.frame()?.parentFrame();
          if (fromView && !request.url().startsWith("data:")) networkAttempts.push(request.url().slice(0, 200));
        });
        const shot = join(out, "screenshots", `${entry.testCase.id}-${theme}.png`);
        try {
          await page.goto(`http://localhost:${port}/scripts/eval-personal-apps/host.html?id=${entry.testCase.id}&theme=${theme}`);
          await page.locator("[data-eval-host] iframe").waitFor({ timeout: 20000 });
          await page.waitForTimeout(9000);
          const view = page.frames().filter(frame => frame.url().startsWith("about:srcdoc") && frame.childFrames().length === 0).at(-1);
          const text = view ? (await view.locator("body").innerText().catch(() => "")).trim() : "";
          // Controls count as content: a form with a field and one button
          // has little text and still rendered.
          const controls = view ? await view.locator("input, textarea, select, button, img, svg, canvas").count().catch(() => 0) : 0;
          // How light the view's own background is, 0 (black) to 1 (white):
          // a view that follows Bridge's theme is lighter in light than dark.
          const luminance = view ? await view.evaluate(() => {
            const parse = value => (value.match(/[\d.]+/g) ?? []).map(Number);
            // The largest element that paints a background: views often
            // color a wrapper, not <body>.
            let best = null, area = 0;
            for (const node of [document.documentElement, document.body, ...document.body.querySelectorAll("*")]) {
              const [r, g, b, a = 1] = parse(getComputedStyle(node).backgroundColor);
              if (!(a > 0.1) || r === undefined) continue;
              const box = node.getBoundingClientRect();
              const size = box.width * box.height;
              if (size > area) { area = size; best = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255; }
            }
            return best;
          }).catch(() => null) : null;
          // The text's own lightness: a transparent view that lets Bridge's
          // card show through still follows the theme if its text flips.
          const textLuminance = view ? await view.evaluate(() => {
            const [r, g, b] = (getComputedStyle(document.body).color.match(/[\d.]+/g) ?? []).map(Number);
            return r === undefined ? null : (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
          }).catch(() => null) : null;
          await page.locator("[data-eval-host]").screenshot({ path: shot });
          if (theme === "light" && view) {
            // One interaction, as a person would try: type the sample query
            // (or a plain word) into the first empty text field, then press
            // the first button.
            // One interaction, as a person would try it: type the sample query
            // (or a plain word) into the first visible text field and press
            // Enter; if that calls nothing, press the button nearest the
            // field (not a tab at the top), else the first button.
            const args = entry.manifest.sampleArgs ?? {};
            const queryKey = ["query", "q", "search", "term", "text", "url", "link"].find(key => typeof args[key] === "string");
            const sample = queryKey ? args[queryKey] : "test";
            const field = view.locator("input:not([type]):visible, input[type=text]:visible, input[type=search]:visible, input[type=url]:visible, textarea:visible").first();
            const before = entry.viewCalls?.length ?? 0;
            if (await field.count()) {
              if (!(await field.inputValue().catch(() => ""))) await field.fill(sample).catch(() => undefined);
              await field.press("Enter").catch(() => undefined);
              await page.waitForTimeout(6000);
            }
            if ((entry.viewCalls?.length ?? 0) === before) {
              const near = (await field.count()) ? field.locator("xpath=ancestor::*[.//button][1]//button[not(@disabled)]").last() : null;
              const button = near && await near.count() ? near : view.locator("button:not([disabled]):visible").first();
              if (await button.count()) await button.click({ timeout: 3000 }).catch(() => undefined);
            }
            await page.waitForTimeout(12000);
            await page.locator("[data-eval-host]").screenshot({ path: join(out, "screenshots", `${entry.testCase.id}-light-after-click.png`) });
          }
          entry.render[theme] = { ok: text.length > 0 || controls > 0, textLength: text.length, controls, luminance, textLuminance, networkAttempts, cspViolations, screenshot: shot };
        } catch (error) {
          entry.render[theme] = { ok: false, error: String(error.message ?? error).slice(0, 200), networkAttempts, cspViolations };
        }
        await context.close();
      }
      log(`render ${entry.testCase.id}: light ${entry.render.light?.ok ? "ok" : "FAIL"}, dark ${entry.render.dark?.ok ? "ok" : "FAIL"}, ${entry.viewCalls?.length ?? 0} view call(s)`);
    }
  } finally {
    await browser.close();
    await server.close();
  }
}

async function lock(entries) {
  for (const entry of entries.filter(item => item.manifest)) {
    const earlier = lockedAs(entry.contentHash);
    if (earlier) {
      entry.lock = { ok: true, kind: "locked", appId: earlier };
      continue;
    }
    try {
      const result = await call("apps/lock_personal_app", { draftId: entry.draftId, contentHash: entry.contentHash, allowWider: false });
      entry.lock = { ok: result.kind === "locked" || result.kind === "unchanged", kind: result.kind, appId: result.app?.id };
    } catch (error) {
      entry.lock = { ok: false, error: String(error.message ?? error) };
    }
  }
}

async function routing(entries) {
  // Chats default to asking first; a direct call to a real server still
  // only ever reads, since every ask is read-only.
  const defaults = await call("config/get_chat_defaults", { projectId: null });
  await call("config/save_chat_defaults", { projectId: null, expectedRevision: defaults.application.revision, defaults: { ...defaults.application.defaults, access: "ask" } }).catch(error => log("could not default chats to Ask:", error.message));
  const appsFor = lockedAppsByCase(entries);
  const jobs = chats.flatMap(chat => ROUTING_ASKS.map(ask => ({ chat, ...ask })));
  const rows = await pool(jobs, async job => {
    const expectAppId = job.expect === null ? null : appsFor.get(job.expect);
    if (job.expect !== null && !expectAppId?.length) return { ...job, expectAppId: null, skipped: `no locked app for ${job.expect}`, result: { verdict: "skipped", opened: [], direct: [] } };
    const events = await turn(job.chat, job.ask, join(out, "routing", `${job.chat}-${job.id}.jsonl`));
    const result = classifyRouting(events, expectAppId, APP_SERVERS);
    log(`route ${job.chat} ${job.id}: ${result.verdict}${result.opened.length ? ` (${result.opened.join(", ")})` : ""}`);
    return { ...job, expectAppId, result };
  });
  return { rows: rows.filter(row => !row.skipped), skipped: rows.filter(row => row.skipped) };
}

// --- rescore -----------------------------------------------------------------

/** Score a finished run again from its saved event logs, with the current
 *  scorer: `--rescore <results dir>`. Nothing is run. */
async function rescore(dir) {
  const { readFileSync, readdirSync } = await import("node:fs");
  const read = file => readFileSync(file, "utf8").trim().split("\n").filter(Boolean).flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } });
  const saved = JSON.parse(readFileSync(join(dir, "results.json"), "utf8"));
  const builds = saved.entries.map(entry => {
    const outcome = withRecordedDraft(buildOutcome(read(join(dir, "build", `${entry.testCase.id}.jsonl`))), saved.dataDir);
    return scoreBuild(entry.testCase, { ...entry, outcome, manifest: entry.manifest });
  });
  const appsFor = lockedAppsByCase(saved.entries);
  const rows = [];
  for (const file of readdirSync(join(dir, "routing")).filter(name => name.endsWith(".jsonl")).sort()) {
    const [chat, ...rest] = file.replace(/\.jsonl$/, "").split("-");
    const ask = ROUTING_ASKS.find(item => item.id === rest.join("-"));
    if (!ask) continue;
    const expectAppId = ask.expect === null ? null : appsFor.get(ask.expect) ?? [];
    rows.push({ chat, ...ask, expectAppId, result: classifyRouting(read(join(dir, "routing", file)), expectAppId, APP_SERVERS) });
  }
  const routed = { rows };
  writeFileSync(join(dir, "results.json"), JSON.stringify({ ...saved, builds, routing: { rows, summary: routingSummary(rows) } }, null, 2));
  writeFileSync(join(dir, "report.md"), markdownReport({ builds, routing: routed }));
  console.log(markdownReport({ builds, routing: routed }));
}

// --- resume ------------------------------------------------------------------

/** Finish a run again on its own data directory: `--resume <results dir>`.
 *  Drafts the events missed (OpenCode's code mode) are taken from Bridge's
 *  record, then the read, render and lock phases run again with the current
 *  runner, and everything is rescored. Routing results are kept. */
async function resume(dir, daemonStarted) {
  const { readFileSync } = await import("node:fs");
  const saved = JSON.parse(readFileSync(join(dir, "results.json"), "utf8"));
  const read = file => readFileSync(file, "utf8").trim().split("\n").filter(Boolean).flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } });
  const entries = [];
  for (const old of saved.entries) {
    const outcome = withRecordedDraft(buildOutcome(read(join(dir, "build", `${old.testCase.id}.jsonl`))));
    const entry = { testCase: old.testCase, outcome, ms: old.ms };
    if (outcome.drafted) {
      entry.draftId = outcome.draft.draftId;
      entry.contentHash = outcome.draft.contentHash;
      entry.sessionId = outcome.sessionId;
      entry.manifest = manifestOf(entry.draftId, entry.contentHash);
      for (const tool of entry.manifest?.reads ?? []) {
        entry.liveRead = await preview(entry, tool, entry.manifest.sampleArgs ?? {});
        if (entry.liveRead.ok && !entry.liveRead.isError) break;
      }
    }
    entries.push(entry);
  }
  await render(entries);
  await lock(entries);
  const builds = entries.map(entry => scoreBuild(entry.testCase, entry));
  const results = {
    ...saved,
    builds,
    entries: entries.map(({ manifest, ...entry }) => ({ ...entry, manifest: manifest && { ...manifest, viewHtml: `${manifest.viewHtml.length} bytes` } })),
  };
  writeFileSync(join(dir, "results.json"), JSON.stringify(results, null, 2));
  daemonStarted.kill();
  // Routing is classified again from its saved events with the current
  // scorer, against the apps locked now.
  await rescore(dir);
}

// --- main --------------------------------------------------------------------

if (flag("rescore", undefined)) {
  await rescore(flag("rescore"));
  process.exit(0);
}

log(`data ${dataDir}`);
log(`results ${out}`);
const daemon = await startDaemon();
if (resumeDir) {
  await resume(resumeDir, daemon);
  process.exit(0);
}
try {
  const cases = BUILD_CASES.filter(item => !ids || ids.includes(item.id));
  const entries = phases.has("build") ? await pool(cases, build) : [];
  if (phases.has("render")) await render(entries);
  if (phases.has("lock")) await lock(entries);
  const routed = phases.has("routing") ? await routing(entries) : null;
  const builds = entries.map(entry => scoreBuild(entry.testCase, entry));
  const results = {
    dataDir,
    builds,
    entries: entries.map(({ manifest, ...entry }) => ({ ...entry, manifest: manifest && { ...manifest, viewHtml: `${manifest.viewHtml.length} bytes` } })),
    routing: routed && { ...routed, summary: routingSummary(routed.rows) },
  };
  writeFileSync(join(out, "results.json"), JSON.stringify(results, null, 2));
  writeFileSync(join(out, "report.md"), markdownReport({ builds, routing: routed }));
  console.log(markdownReport({ builds, routing: routed }));
} finally {
  daemon.kill();
}
