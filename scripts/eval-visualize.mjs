#!/usr/bin/env node
// Run the visualize eval against a real harness.
//
//   bun run eval:visualize -- --harness claude [--model sonnet] [--limit 40]
//                             [--category render] [--ids render-001,quiet-004]
//                             [--concurrency 3] [--server path/to/bridged]
//
// Each case runs in a fresh, empty working directory with only Bridge's
// `bridge` MCP server attached, the chat rendering note Bridge sends, and no
// file, shell or web tools, so the only decision under test is whether and
// how to call `visualize`. Prior turns of a multi-turn case are given as a
// transcript in the prompt. Results land in testing/evals/visualize/results/.
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { aggregate, REFUSAL_PREFIX, scoreCase } from "./eval-visualize/score.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const harness = flag("harness", "claude");
const model = flag("model", undefined);
const limit = Number(flag("limit", "0"));
const concurrency = Number(flag("concurrency", "3"));
const category = flag("category", undefined);
const ids = flag("ids", undefined)?.split(",");

function resolveServer() {
  const explicit = flag("server", process.env.BRIDGE_VISUALIZE_MCP_BIN);
  if (explicit) return explicit;
  const target = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps", "--manifest-path", join(root, "src-tauri/Cargo.toml")], { encoding: "utf8" })).target_directory;
  const binary = join(target, "debug", "bridged");
  if (!existsSync(binary)) throw new Error(`build it first: cargo build -p bridged (looked for ${binary})`);
  return binary;
}

/** The chat rendering note exactly as prompts.rs defines it. */
function renderingNote() {
  const source = readFileSync(join(root, "src-tauri/bridge-core/src/prompts.rs"), "utf8");
  const literal = name => {
    const match = new RegExp(`pub const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`, "s").exec(source);
    if (!match) throw new Error(`${name} not found in prompts.rs`);
    return match[1].replace(/\\n/g, "\n").replace(/\\"/g, '"').replace(/\\\\/g, "\\");
  };
  return `${literal("RENDERING_NOTE")}\n\n${literal("VISUALIZE_NOTE")}`;
}

function promptFor(testCase) {
  const turns = testCase.turns;
  const last = turns[turns.length - 1].text;
  if (turns.length === 1) return last;
  const history = turns.slice(0, -1).map(turn => `${turn.role === "user" ? "User" : "Assistant"}: ${turn.text}`).join("\n\n");
  return `Conversation so far (a line in [brackets] is a visual already shown to the user):\n\n${history}\n\nThe user now says:\n${last}`;
}

function run(command, commandArgs, options) {
  return new Promise(resolve => {
    const child = spawn(command, commandArgs, { ...options, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", chunk => { stdout += chunk; });
    child.stderr.on("data", chunk => { stderr += chunk; });
    const timer = setTimeout(() => child.kill("SIGKILL"), 240_000);
    child.on("close", code => { clearTimeout(timer); resolve({ code, stdout, stderr }); });
  });
}

const lines = text => text.split("\n").map(line => { try { return JSON.parse(line); } catch { return null; } }).filter(Boolean);
const textOf = content => Array.isArray(content) ? content.map(part => part?.text ?? "").join("") : typeof content === "string" ? content : "";

async function observeClaude(testCase, server, note, cwd) {
  const config = JSON.stringify({ mcpServers: { bridge: { type: "stdio", command: server, args: ["--bridge-mcp-visualize"] } } });
  const commandArgs = ["-p", promptFor(testCase), "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config", config,
    "--append-system-prompt", note, "--setting-sources", "project", "--allowedTools", "mcp__bridge__visualize",
    "--disallowedTools", "Bash,Edit,Write,MultiEdit,NotebookEdit,WebFetch,WebSearch,Task,Agent", "--max-turns", "8"];
  if (model) commandArgs.push("--model", model);
  const { code, stdout, stderr } = await run("claude", commandArgs, { cwd });
  const calls = new Map();
  let reply = "";
  let result;
  for (const event of lines(stdout)) {
    if (event.type === "result") result = event;
    for (const part of event.message?.content ?? []) {
      if (part.type === "tool_use" && part.name === "mcp__bridge__visualize") calls.set(part.id, { input: part.input, refused: false });
      if (part.type === "tool_result" && calls.has(part.tool_use_id)) {
        const call = calls.get(part.tool_use_id);
        call.refused = part.is_error === true || textOf(part.content).startsWith(REFUSAL_PREFIX);
        if (call.refused) call.refusal = textOf(part.content);
      }
      if (part.type === "text" && event.type === "assistant") reply = part.text;
    }
  }
  // A run that never produced a successful result is the harness failing,
  // not the model deciding: it must not score as "chose not to draw".
  const error = code !== 0 || !result || result.is_error || String(result.subtype ?? "").startsWith("error")
    ? `exit ${code}; ${result ? `${result.subtype}: ${String(result.result ?? "").slice(0, 200)}` : "no result event"}; ${stderr.slice(0, 200)}`
    : undefined;
  return { calls: [...calls.values()], reply, error };
}

async function observeCodex(testCase, server, note, cwd) {
  const commandArgs = ["exec", "--json", "--skip-git-repo-check", "-s", "read-only", "-c", 'approval_policy="never"',
    "-c", `mcp_servers.bridge.command="${server.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`, "-c", 'mcp_servers.bridge.args=["--bridge-mcp-visualize"]',
    "-c", "plugins.visualize@openai-bundled.enabled=false", "-c", `developer_instructions=${JSON.stringify(note)}`];
  if (model) commandArgs.push("-m", model);
  commandArgs.push(promptFor(testCase));
  const { code, stdout, stderr } = await run("codex", commandArgs, { cwd });
  const calls = new Map();
  let reply = "";
  let completed = false;
  let failure = "";
  for (const event of lines(stdout)) {
    if (event.type === "turn.completed") completed = true;
    if (event.type === "turn.failed" || event.type === "error") failure = JSON.stringify(event).slice(0, 200);
    const item = event.item ?? {};
    if (item.type === "mcp_tool_call" && item.server === "bridge" && item.tool === "visualize" && event.type === "item.completed") {
      const text = textOf(item.result?.content);
      const refused = !!item.error || text.startsWith(REFUSAL_PREFIX);
      calls.set(item.id, { input: item.arguments, refused, ...(refused ? { refusal: text || JSON.stringify(item.error) } : {}) });
    }
    if (item.type === "agent_message" && event.type === "item.completed") reply = item.text ?? "";
  }
  const error = code !== 0 || !completed || failure ? `exit ${code}; ${failure || "no turn.completed"}; ${stderr.slice(-200)}` : undefined;
  return { calls: [...calls.values()], reply, error };
}

async function observeOpenCode(testCase, server, note, cwd) {
  const config = JSON.stringify({ mcp: { bridge: { type: "local", command: [server, "--bridge-mcp-visualize"], enabled: true } }, instructions: [] });
  const commandArgs = ["run", "--format", "json"];
  if (model) commandArgs.push("--model", model);
  commandArgs.push(`${note}\n\n${promptFor(testCase)}`);
  const { code, stdout, stderr } = await run("opencode", commandArgs, { cwd, env: { ...process.env, OPENCODE_CONFIG_CONTENT: config } });
  const calls = [];
  let reply = "";
  let failure = "";
  for (const event of lines(stdout)) {
    if (event.type === "error") failure = JSON.stringify(event).slice(0, 200);
    const part = event.part ?? {};
    if (part.type === "tool" && part.tool === "bridge_visualize" && part.state?.status !== "running") {
      const refused = part.state?.status === "error" || String(part.state?.output ?? "").startsWith(REFUSAL_PREFIX);
      calls.push({ input: part.state?.input, refused, ...(refused ? { refusal: String(part.state?.output ?? part.state?.error ?? "") } : {}) });
    }
    if (part.type === "text") reply = part.text ?? reply;
  }
  const error = code !== 0 || failure ? `exit ${code}; ${failure}; ${stderr.slice(-200)}` : undefined;
  return { calls, reply, error };
}

const observe = { claude: observeClaude, codex: observeCodex, opencode: observeOpenCode }[harness];
if (!observe) throw new Error(`unknown harness ${harness}`);

let cases = readFileSync(join(root, "testing/evals/visualize/cases.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line));
if (category) cases = cases.filter(testCase => testCase.category === category);
if (ids) cases = cases.filter(testCase => ids.includes(testCase.id));
if (limit > 0) {
  // Stratified: round-robin across categories so a sample keeps the mix.
  const groups = new Map();
  for (const testCase of cases) groups.set(testCase.category, [...(groups.get(testCase.category) ?? []), testCase]);
  const picked = [];
  for (let round = 0; picked.length < limit && [...groups.values()].some(group => group.length > round); round += 1) {
    for (const group of groups.values()) if (group[round] && picked.length < limit) picked.push(group[round]);
  }
  cases = picked;
}

const server = resolveServer();
const note = renderingNote();
const results = [];
const observations = [];
const errored = [];
let next = 0;
async function worker() {
  while (next < cases.length) {
    const testCase = cases[next++];
    const cwd = mkdtempSync(join(tmpdir(), "visualize-eval-"));
    const started = Date.now();
    let observation;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      if (attempt > 0) await new Promise(resolve => setTimeout(resolve, 15_000 * attempt));
      observation = await observe(testCase, server, note, cwd);
      if (!observation.error) break;
    }
    observations.push({ id: testCase.id, ms: Date.now() - started, ...observation });
    if (observation.error) {
      errored.push({ id: testCase.id, error: observation.error });
      console.log(`ERR  ${testCase.id.padEnd(12)} ${observation.error.slice(0, 120)}`);
      continue;
    }
    const result = scoreCase(testCase, observation);
    results.push(result);
    const mark = result.decisionCorrect ? "ok " : "MISS";
    console.log(`${mark} ${testCase.id.padEnd(12)} called=${result.called} forms=${result.forms.join("+") || "-"}${result.firstTryValid === false ? " refused-first" : ""}${result.untraced.length ? ` untraced=${result.untraced.length}` : ""}`);
  }
}
await Promise.all(Array.from({ length: Math.max(1, concurrency) }, worker));

const summary = aggregate(results, cases);
const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
const outDir = join(root, "testing/evals/visualize/results");
mkdirSync(outDir, { recursive: true });
const outFile = join(outDir, `${harness}-${model ?? "default"}-${stamp}.json`);
writeFileSync(outFile, JSON.stringify({ harness, model: model ?? "default", cases: cases.length, errored, summary, results, observations }, null, 2) + "\n");
console.log(`\n${results.length} scored, ${errored.length} errored after retries (excluded)`);
console.log("\n| Metric | Value | Gate | Pass |\n|---|---|---|---|");
const show = value => (value === null ? "n/a" : typeof value === "number" && value <= 1 && !Number.isInteger(value) ? value.toFixed(3) : String(value));
const gates = { precision: "≥ 0.95", recall: "≥ 0.90", formFit: "≥ 0.90", firstTryValid: "≥ 0.95", validAfterRetry: "≥ 0.995", untraced: "0", traps: "≥ 0.95", repeats: "0", overuse: "≤ 1.5×" };
for (const [key, gate] of Object.entries(gates)) {
  console.log(`| ${key} | ${show(summary.metrics[key])}${key === "overuse" && summary.metrics.overused ? ` (${summary.metrics.overused})` : ""} | ${gate} | ${summary.pass[key] ? "yes" : "NO"} |`);
}
console.log(`\n${outFile}`);
