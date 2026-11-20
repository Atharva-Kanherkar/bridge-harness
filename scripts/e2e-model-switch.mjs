#!/usr/bin/env node
// Live end-to-end check of the model-switch handoff against the real app.
//
// Runs the branch-built `bridged` on a scratch data directory, with the real
// Claude Code and Codex logins, and drives it through `bridge exec --method`,
// the same protocol methods the desktop app calls. Three journeys:
//
//   tools    Claude writes a file and prints a PR URL. Its runtime is stopped
//            (by the idle reaper, so no summary can be asked for), the chat
//            switches to Codex, and Codex is asked what was written and linked.
//            Asserts: the switch notice says "(no model summary)", the cold
//            start is checkpoint_restored with a restoration_context section,
//            and Codex names the file and the PR.
//   history  A Claude chat is told a code word, then given more text than any
//            handoff budget carries. After the switch to Codex, Codex is asked
//            for the code word. Asserts: Codex called chats_history and
//            answered with the word.
//   carry    A Claude chat on your own login is told a code word, then
//            switched to the `personal` account (~/.claude_personal). Asserts:
//            the transcript was copied there, the thread id is unchanged, no
//            resume failed, and the chat still knows the word. Then back to
//            your own login, the same again. The scratch project folders are
//            removed from both homes afterwards.
//
//   cargo build -p bridged -p bridge-client --bins
//   E2E_BIN_DIR=<target>/debug node scripts/e2e-model-switch.mjs [tools|history|carry ...]
//
// Writes a JSON report to $E2E_REPORT (default <data dir>/report.json).

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// E2E_BIN_DIR, else the cargo target's debug folder. Building the Tauri shell
// stages placeholder sidecars over `bridged` there, so copying the built
// binaries aside first is the safer habit.
const target = process.env.CARGO_TARGET_DIR ?? join(process.cwd(), "src-tauri/target");
const binDir = process.env.E2E_BIN_DIR ?? join(target, "debug");
const bin = (name) => join(binDir, name);
const data = process.env.E2E_DATA ?? join(homedir(), ".cache/bridge-e2e-model-switch", `run-${Date.now()}`);
const reportPath = process.env.E2E_REPORT ?? join(data, "report.json");
// Matched against the live catalog, which starts with aliases (`haiku`) and
// is replaced by full ids (`claude-haiku-4-5-…`) once the CLI answers.
const claudePattern = new RegExp(process.env.E2E_CLAUDE_MODEL ?? "haiku");
const codexPattern = new RegExp(process.env.E2E_CODEX_MODEL ?? "gpt-5\\.6-luna");
const journeys = process.argv.slice(2).length ? process.argv.slice(2) : ["tools", "history", "carry"];
const report = { data, journeys: {} };

/// One protocol method (`domain/command`), as the desktop app calls it.
function call(method, params, timeout = 600) {
  const args = ["exec", "--json", "--data-dir", data, "--timeout", String(timeout), "--method", method];
  if (params !== undefined) args.push("--params", JSON.stringify(params));
  const run = spawnSync(bin("bridge"), args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  const line = run.stdout.trim().split("\n").filter(Boolean).pop();
  if (!line) throw new Error(`${method}: no output\n${run.stderr}`);
  const parsed = JSON.parse(line);
  if (parsed.type !== "result") throw new Error(`${method}: ${JSON.stringify(parsed).slice(0, 2000)}`);
  return parsed.result;
}

function sql(query) {
  const run = spawnSync("sqlite3", ["-readonly", "-json", `file:${join(data, "bridge.db")}?mode=ro`, query], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(run.stderr);
  return run.stdout.trim() ? JSON.parse(run.stdout) : [];
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function entries(sessionId) {
  return sql(`SELECT sequence, kind, payload FROM session_entries WHERE session_id='${sessionId}' ORDER BY sequence`).map((row) => ({ ...row, payload: JSON.parse(row.payload) }));
}

const newest = (sessionId) => sql(`SELECT coalesce(max(sequence),0) AS n FROM session_entries WHERE session_id='${sessionId}'`)[0].n;

/// The catalog's current id for a model matching `pattern`.
function model(harness, pattern) {
  const adapter = (call("health/health").adapters ?? []).find((candidate) => candidate.id === harness);
  return adapter?.models.find((option) => option.available && pattern.test(option.id))?.id ?? null;
}

/// A switch, retried while another lifecycle operation (the idle release
/// that just stopped the runtime) still holds the chat, as a person would.
async function switchTo(params) {
  for (let tries = 0; ; tries += 1) {
    try {
      return call("sessions/update_chat_model", params);
    } catch (error) {
      if (!/Another operation/.test(String(error)) || tries > 30) throw error;
      await sleep(2000);
    }
  }
}

/// Until the turn sent after `mark` ends: a turn.completed (or an error) after
/// it, and the chat no longer working.
async function waitTurn(sessionId, mark, label, timeoutMs = 900_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const pending = sql(`SELECT count(*) AS n FROM session_entries WHERE session_id='${sessionId}' AND sequence>${mark} AND kind='approval.requested'`)[0].n;
    if (pending > 0) throw new Error(`${label}: the chat asked for approval; it should run with full access`);
    const [row] = sql(`SELECT status FROM sessions WHERE id='${sessionId}'`);
    const ended = sql(`SELECT kind FROM session_entries WHERE session_id='${sessionId}' AND sequence>${mark} AND kind IN ('turn.completed','turn.failed','error','runtime.failed')`);
    if (ended.length && row?.status !== "working") {
      if (ended.some((entry) => entry.kind !== "turn.completed")) console.log(`  (${label}: ${ended.map((entry) => entry.kind).join(",")})`);
      return;
    }
    await sleep(2000);
  }
  throw new Error(`${label}: the turn did not finish`);
}

async function say(sessionId, text, label) {
  const mark = newest(sessionId);
  call("sessions/send_turn", { sessionId, text });
  await waitTurn(sessionId, mark, label);
  const replies = entries(sessionId).filter((entry) => entry.sequence > mark && entry.kind === "assistant.message");
  const reply = replies.map((entry) => entry.payload.text ?? "").join("\n");
  console.log(`  ${label}: ${reply.replace(/\s+/g, " ").slice(0, 160)}`);
  return reply;
}

async function newChat(harness, model) {
  const { sessionId } = call("sessions/create_chat_id", { harness, model, title: `e2e ${harness}` });
  const settings = call("sessions/get_chat_settings", { sessionId });
  call("sessions/update_chat_options", { sessionId, expectedRevision: settings.revision, access: "full", activeTurnInput: settings.settings.activeTurnInput });
  return sessionId;
}

async function untilReaped(sessionId) {
  // The idle reaper stops a runtime after two minutes; after that no outgoing
  // model is alive to summarise, which is the switch this change is for.
  const start = Date.now();
  while (Date.now() - start < 6 * 60_000) {
    const [row] = sql(`SELECT adapter_pid FROM sessions WHERE id='${sessionId}'`);
    if (!row || !row.adapter_pid) return Math.round((Date.now() - start) / 1000);
    await sleep(5000);
  }
  throw new Error("the runtime was never reaped");
}

function switchNotice(sessionId) {
  const changes = entries(sessionId).filter((entry) => entry.kind === "session.model_changed");
  return changes[changes.length - 1]?.payload ?? null;
}

function check(results, name, ok, detail) {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? `: ${String(detail).slice(0, 300)}` : ""}`);
}

async function toolsJourney() {
  const results = [];
  const sessionId = await newChat("claude", model("claude", claudePattern));
  const reply = await say(sessionId, "Use your tools: write a file named notes/plan.md containing exactly the line 'retry budget: 3', then run the shell command `echo https://github.com/acme/demo/pull/4242`. Then reply with one short sentence.", "claude tools turn");
  check(results, "claude ran the tools", entries(sessionId).some((entry) => entry.kind === "file_change.completed" || entry.kind === "tool.completed"), reply);
  const waited = await untilReaped(sessionId);
  check(results, "the outgoing runtime is gone before the switch", true, `${waited}s`);
  await switchTo({ sessionId, harness: "codex", model: model("codex", codexPattern) });
  const notice = switchNotice(sessionId);
  check(results, "the notice says what Bridge carried with no model summary", /no model summary/.test(notice?.text ?? "") && /file/.test(notice?.text ?? ""), notice?.text);
  check(results, "no compaction entry is written", !entries(sessionId).some((entry) => entry.kind === "compaction"));
  const answer = await say(sessionId, "Without running any tools, answer from what you were handed: which file was written earlier in this chat, and which pull request URL was printed? Reply exactly as: file=<path> pr=<url>", "codex answer");
  check(results, "codex names the file", /notes\/plan\.md/.test(answer), answer);
  check(results, "codex names the PR", /github\.com\/acme\/demo\/pull\/4242/.test(answer), answer);
  const [ledger] = sql(`SELECT restoration_mode, sections_json, variable_bytes FROM prompt_compilations WHERE session_id='${sessionId}' AND harness='codex' ORDER BY id LIMIT 1`);
  check(results, "the cold start is checkpoint_restored with the handoff section", ledger?.restoration_mode === "checkpoint_restored" && /restoration_context/.test(ledger?.sections_json ?? ""), JSON.stringify(ledger));
  return { sessionId, results };
}

async function historyJourney() {
  const results = [];
  const sessionId = await newChat("claude", model("claude", claudePattern));
  // The first ask rides in the handoff's `started with:` line, so the fact
  // goes in the second message, where only chats_history can reach it.
  await say(sessionId, "Let's plan the Juniper release together. Reply only with 'ok'.", "first ask");
  await say(sessionId, "For the release notes: the staging port for Juniper is 8443. Reply only with 'ok'.", "the fact");
  // More than any handoff budget carries (96 KB, 16 KB per message).
  const filler = (index) => `Log batch ${index} from the release dry run, no action needed, reply only with 'noted'.\n` + Array.from({ length: 220 }, (_, line) => `line ${index}.${line}: retry scheduler tick ok, queue depth nominal, nothing to report`).join("\n");
  for (let index = 0; index < 8; index += 1) await say(sessionId, filler(index), `filler ${index}`);
  await untilReaped(sessionId);
  await switchTo({ sessionId, harness: "codex", model: model("codex", codexPattern) });
  const before = newest(sessionId);
  const answer = await say(sessionId, "What staging port did I give you for the Juniper release earlier in this chat? If it is not in what you were handed, look it up. Reply with just the number.", "codex port");
  const after = entries(sessionId).filter((entry) => entry.sequence > before);
  const lookedUp = after.some((entry) => /chats_history/.test(JSON.stringify(entry.payload)));
  check(results, "codex looked it up with chats_history", lookedUp, after.map((entry) => entry.kind).join(","));
  check(results, "codex answered with the port", /\b8443\b/.test(answer) && answer.length < 60, answer);
  return { sessionId, results };
}

function claudeFolders(sessionId) {
  const [row] = sql(`SELECT cwd, provider_session_id FROM sessions WHERE id='${sessionId}'`);
  const folder = (row.cwd ?? "").replace(/[^A-Za-z0-9]/g, "-");
  return { thread: row.provider_session_id, own: join(homedir(), ".claude/projects", folder), personal: join(homedir(), ".claude_personal/projects", folder) };
}

async function carryJourney() {
  const results = [];
  const config = call("config/get_config_state");
  const claude = config.harnesses.find((harness) => harness.id === "claude");
  delete claude.isOverride;
  claude.advanced = { ...(claude.advanced ?? {}), accounts: [{ id: "personal", label: "Personal", homePath: "~/.claude_personal" }] };
  call("config/save_harness_config", { config: claude });
  const sessionId = await newChat("claude", model("claude", claudePattern));
  await say(sessionId, "Some context about me: my dog's name is Biscuit. Reply only with 'noted'.", "the fact");
  const { thread, own, personal } = claudeFolders(sessionId);
  const ownFile = join(own, `${thread}.jsonl`);
  const personalFile = join(personal, `${thread}.jsonl`);
  // A short, exact answer: a refusal that happens to quote the word is not one.
  const knows = (reply) => /\bBiscuit\b/.test(reply) && reply.length < 40;
  try {
    check(results, "the thread is on disk under your own login", existsSync(ownFile), ownFile);
    await switchTo({ sessionId, harness: "claude", model: model("claude", claudePattern), account: "personal" });
    const notice = switchNotice(sessionId);
    check(results, "the notice says the thread was carried", /carried to the new account/.test(notice?.text ?? "") && notice?.data?.carriedThread === true, notice?.text);
    check(results, "the transcript is in the personal home", existsSync(personalFile), personalFile);
    const [moved] = sql(`SELECT account, provider_session_id FROM sessions WHERE id='${sessionId}'`);
    check(results, "the thread id is unchanged", moved.provider_session_id === thread && moved.account === "personal", JSON.stringify(moved));
    const answer = await say(sessionId, "What is my dog's name? Reply with just the name.", "personal answer");
    check(results, "the carried thread still knows the fact", knows(answer), answer);
    check(results, "no resume failed", !entries(sessionId).some((entry) => entry.kind === "session.resume_failed"));
    const [ledger] = sql(`SELECT restoration_mode FROM prompt_compilations WHERE session_id='${sessionId}' ORDER BY id DESC LIMIT 1`);
    check(results, "the start resumed natively", ledger?.restoration_mode === "native", JSON.stringify(ledger));
    await switchTo({ sessionId, harness: "claude", model: model("claude", claudePattern), account: "claude" });
    const copiedBack = existsSync(ownFile) && readFileSync(ownFile, "utf8").includes("What is my dog's name?");
    check(results, "the way back copies the longer transcript home", copiedBack, ownFile);
    const back = await say(sessionId, "And my dog's name once more? Reply with just the name.", "own answer");
    check(results, "back on your own login it still knows it", knows(back), back);
    const [home] = sql(`SELECT account, provider_session_id FROM sessions WHERE id='${sessionId}'`);
    check(results, "same thread after the way back", home.provider_session_id === thread && home.account === null, JSON.stringify(home));
    check(results, "no resume failed on the way back", !entries(sessionId).some((entry) => entry.kind === "session.resume_failed"));
  } finally {
    for (const folder of [own, personal]) {
      if (folder.endsWith("projects/") || !folder.includes("bridge-e2e-model-switch")) continue;
      rmSync(folder, { recursive: true, force: true });
    }
  }
  return { sessionId, results };
}

mkdirSync(data, { recursive: true });
// Your own login is what the desktop app inherits: no CLAUDE_CONFIG_DIR or
// CODEX_HOME from a terminal that happens to set one.
const env = { ...process.env, BRIDGE_VISUALIZE_MCP_BIN: bin("bridged") };
delete env.CLAUDE_CONFIG_DIR;
delete env.CODEX_HOME;
const daemon = spawn(bin("bridged"), ["--data-dir", data, "--health-addr", "none"], { stdio: ["ignore", "inherit", "inherit"], env });
let failed = 0;
try {
  for (let tries = 0; !existsSync(join(data, "bridged.sock")); tries += 1) {
    // Without a daemon, `bridge exec` would host the runtime itself for one
    // call and stop every turn when it exits.
    if (daemon.exitCode !== null || tries > 120) throw new Error(`bridged did not start (exit ${daemon.exitCode})`);
    await sleep(500);
  }
  // Model catalogs are discovered in the background; a chat on a model the
  // catalog does not list yet is refused.
  for (let tries = 0; ; tries += 1) {
    const adapters = call("health/health").adapters ?? [];
    const lists = (id, pattern) => adapters.some((adapter) => adapter.id === id && adapter.models.some((option) => pattern.test(option.id) && option.available));
    // Wait for the settled catalog too: it starts with aliases and swaps them
    // for full ids within minutes, and a chat created on an alias just before
    // the swap is refused at its first send.
    const settled = adapters.some((adapter) => adapter.id === "claude" && adapter.models.some((option) => /^claude-/.test(option.id)));
    if (lists("claude", claudePattern) && lists("codex", codexPattern) && (settled || tries > 90)) break;
    if (tries > 120) throw new Error(`the catalog never listed claude ${claudePattern} and codex ${codexPattern}`);
    await sleep(2000);
  }
  for (const journey of journeys) {
    const run = { tools: toolsJourney, history: historyJourney, carry: carryJourney }[journey];
    try {
      const outcome = await run();
      report.journeys[journey] = outcome;
      failed += outcome.results.filter((result) => !result.ok).length;
    } catch (error) {
      failed += 1;
      report.journeys[journey] = { error: String(error) };
      console.log(`FAIL ${journey}: ${error}`);
    }
  }
} finally {
  writeFileSync(reportPath, JSON.stringify(report, null, 2));
  daemon.kill("SIGTERM");
  // Claude Code filed this run's chats under a folder named after the scratch
  // data directory, in each home: remove those, and nothing else.
  const slug = data.replace(/[^A-Za-z0-9]/g, "-");
  for (const home of [".claude", ".claude_personal"]) {
    const projects = join(homedir(), home, "projects");
    for (const folder of existsSync(projects) ? readdirSync(projects) : []) {
      if (folder.startsWith(slug)) rmSync(join(projects, folder), { recursive: true, force: true });
    }
  }
}
console.log(`report: ${reportPath}`);
process.exit(failed ? 1 : 0);
