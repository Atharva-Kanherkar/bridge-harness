#!/usr/bin/env node
// Where a chat's time goes between pressing Send and the first token, measured
// from what a real Bridge data directory already recorded.
//
//   node scripts/latency-report.mjs [--since 2026-10-01] [--data-dir <dir>] [--json]
//
// Reads bridge.db and bridge-telemetry.db read-only. The hops it can see:
//
//   new chat   chat.created ─▶ harness launched ─▶ user.message persisted
//   every send user.message ─▶ harness requesting (turn.started) ─▶ first delta
//
// `chat.created` is written when the first message is submitted (#350), so it
// stands in for the Send click on a new chat. Nothing before `user.message` is
// recorded for a follow-up send, and nothing after the daemon persists a frame
// is recorded at all. Those hops come from the `latency.*` ledger rows
// (bridge-core/src/latency.rs), reported under `ledger.*` once a build that
// writes them has run:
//
//   ledger.client.<flow>.<step>    the webview: click to each boundary
//   ledger.rpc.<method>            the daemon: per send-path request, plus response bytes
//   ledger.rpc_window.<method>     the daemon: state reads and slow requests, per minute
//   ledger.launch.<step>           start_chat: resolve, prompt, context, spawn, ...
//   ledger.submit.<step>           submit_input: gates, resume, route, prepare, deliver
//   ledger.harness.<mark>          Claude Code, from the message on stdin

import { DatabaseSync } from "node:sqlite";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : fallback;
};
const dataDir = flag("--data-dir", join(homedir(), "Library/Application Support/dev.bridge.deck"));
const since = flag("--since", new Date(Date.now() - 30 * 86_400_000).toISOString().slice(0, 10));
const asJson = args.includes("--json");
const USER_KINDS = ["direct", "orchestrator"];
// A first delta later than this after a send belongs to something else: a
// queued message, a send into a dead process, a stalled provider. Counted
// separately so one stuck chat cannot become the p99 of a healthy path.
const STALE_MS = 10 * 60_000;

const db = new DatabaseSync(join(dataDir, "bridge.db"), { readOnly: true });
const telemetryPath = join(dataDir, "bridge-telemetry.db");
const telemetry = existsSync(telemetryPath) ? new DatabaseSync(telemetryPath, { readOnly: true }) : null;

const ms = (value) => (value ? Date.parse(value) : NaN);

const samples = new Map();
function add(metric, harness, value) {
  if (!Number.isFinite(value) || value < 0) return;
  for (const key of harness === "-" ? [`${metric}|all`] : [`${metric}|all`, `${metric}|${harness}`]) {
    if (!samples.has(key)) samples.set(key, []);
    samples.get(key).push(value);
  }
}
const counters = new Map();
const count = (name) => counters.set(name, (counters.get(name) ?? 0) + 1);

const sessions = db.prepare(
  `SELECT id, harness, kind FROM sessions
    WHERE kind IN (${USER_KINDS.map(() => "?").join(",")})
      AND id IN (SELECT DISTINCT session_id FROM session_entries
                  WHERE kind='user.message' AND created_at >= ?)`,
).all(...USER_KINDS, since);

const createdAt = new Map(
  db.prepare("SELECT entity_id, MIN(created_at) AS at FROM events WHERE kind='chat.created' GROUP BY entity_id")
    .all().map((row) => [row.entity_id, ms(row.at)]),
);

const ENTRY_KINDS = [
  "user.message", "session.started", "session.status", "turn.started", "turn.completed",
  "assistant.message", "reasoning.started", "reasoning.completed", "tool.started", "command.started",
];
const entriesFor = db.prepare(
  `SELECT sequence, kind, created_at,
          json_extract(payload,'$.status') AS status,
          json_extract(payload,'$.data.subtype') AS subtype
     FROM session_entries
    WHERE session_id=? AND kind IN (${ENTRY_KINDS.map(() => "?").join(",")})
    ORDER BY sequence`,
);
// The first thing the model produced, of any kind. Text alone undercounts: a
// turn that opens with a tool call or a thinking block has already answered.
const OUTPUT_SPANS = [
  "gen_ai.message_delta", "gen_ai.reasoning_delta", "gen_ai.reasoning_started", "gen_ai.message_started",
  "gen_ai.tool_started", "gen_ai.command_started", "gen_ai.file_change_started",
];
const outputsFor = telemetry?.prepare(
  `SELECT started_at FROM telemetry_spans
    WHERE trace_id=? AND name IN (${OUTPUT_SPANS.map(() => "?").join(",")})
    ORDER BY started_at`,
);
const textFor = telemetry?.prepare(
  "SELECT started_at FROM telemetry_spans WHERE trace_id=? AND name='gen_ai.message_delta' ORDER BY started_at",
);

/** First value in a sorted array that is >= `from` and < `to`. */
function firstIn(sorted, from, to) {
  let lo = 0, hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (sorted[mid] < from) lo = mid + 1; else hi = mid;
  }
  return lo < sorted.length && sorted[lo] < to ? sorted[lo] : NaN;
}

const VISIBLE = new Set(["assistant.message", "reasoning.started", "reasoning.completed", "tool.started", "command.started"]);

for (const session of sessions) {
  const harness = session.harness;
  const entries = entriesFor.all(session.id, ...ENTRY_KINDS).map((row) => ({ ...row, at: ms(row.created_at) }));
  const outputs = outputsFor ? outputsFor.all(session.id, ...OUTPUT_SPANS).map((row) => ms(row.started_at)) : [];
  const texts = textFor ? textFor.all(session.id).map((row) => ms(row.started_at)) : [];
  const sinceMs = ms(since);
  // Claude Code's own init frame also maps to session.started; Bridge's launch
  // is the one that is not `init`.
  const isLaunch = (entry) => entry.kind === "session.started" && entry.subtype !== "init";
  let inTurn = false;
  let lastLaunch = NaN;
  let firstUser = true;

  for (let index = 0; index < entries.length; index++) {
    const entry = entries[index];
    if (isLaunch(entry)) { lastLaunch = entry.at; inTurn = false; }
    if (entry.kind === "turn.started") inTurn = true;
    if (entry.kind === "turn.completed") inTurn = false;

    // Two entries the adapter derives from one Claude status frame: whatever
    // separates them is Bridge persisting the first one.
    if (entry.kind === "session.status" && entry.status === "requesting") {
      const next = entries[index + 1];
      if (next?.kind === "turn.started" && next.sequence === entry.sequence + 1 && entry.at >= sinceMs) {
        add("bridge.persist_gap_same_frame", harness, next.at - entry.at);
      }
    }

    if (entry.kind !== "user.message") continue;
    const wasFirst = firstUser;
    firstUser = false;
    if (entry.at < sinceMs) continue;
    if (inTurn) { count("send.steered_or_mid_turn"); continue; }

    let next = Infinity;
    let request = NaN;
    let visible = NaN;
    let init = NaN;
    for (let ahead = index + 1; ahead < entries.length; ahead++) {
      const later = entries[ahead];
      if (later.kind === "user.message") { next = later.at; break; }
      if (Number.isNaN(request) && later.kind === "turn.started") request = later.at;
      if (Number.isNaN(init) && later.kind === "session.started" && later.subtype === "init") init = later.at;
      if (Number.isNaN(visible) && VISIBLE.has(later.kind)) visible = later.at;
    }
    const firstText = firstIn(texts, entry.at, next);
    const firstOutput = Math.min(...[firstIn(outputs, entry.at, next), visible].filter(Number.isFinite));

    if (!Number.isFinite(firstOutput) || firstOutput - entry.at > STALE_MS) { count("send.no_output_or_stale"); continue; }

    const coldBySend = Number.isFinite(lastLaunch) && entry.at - lastLaunch < 60_000;
    const path = wasFirst && createdAt.has(session.id) ? "new" : coldBySend ? "cold" : "warm";
    count(`send.${path}`);

    add(`${path}.send_to_requesting`, harness, request - entry.at);
    add(`${path}.requesting_to_first_output`, harness, firstOutput - request);
    add(`${path}.send_to_first_output`, harness, firstOutput - entry.at);
    add(`${path}.send_to_first_text`, harness, firstText - entry.at);
    if (harness === "claude") add(`${path}.send_to_claude_init`, harness, init - entry.at);

    if (path === "new") {
      const created = createdAt.get(session.id);
      add("new.click_to_harness_launched", harness, lastLaunch - created);
      add("new.harness_launched_to_send_persisted", harness, entry.at - lastLaunch);
      add("new.click_to_send_persisted", harness, entry.at - created);
      add("new.click_to_first_output", harness, firstOutput - created);
    }
    if (path === "cold") add("cold.harness_launched_to_send_persisted", harness, entry.at - lastLaunch);
  }
}

// The latency ledger: every hop the forest cannot show.
const sessionHarness = new Map(db.prepare("SELECT id, harness FROM sessions").all().map((row) => [row.id, row.harness]));
for (const row of db.prepare("SELECT kind, entity_id, body FROM events WHERE source='latency' AND created_at >= ?").iterate(since)) {
  let body;
  try { body = JSON.parse(row.body); } catch { continue; }
  const harness = body.harness ?? sessionHarness.get(row.entity_id) ?? "-";
  const kind = row.kind.replace(/^latency\./, "");
  if (kind === "client") {
    for (const [name, at] of body.steps ?? []) add(`ledger.client.${body.flow}.${name}`, harness, at);
  } else if (kind === "rpc_window") {
    add(`ledger.rpc_window.${body.method}.mean`, harness, body.meanMs);
    add(`ledger.rpc_window.${body.method}.max`, harness, body.maxMs);
    // Calls per minute, shown in the seconds columns: value / 1000 prints calls.
    add(`ledger.rpc_window.${body.method}.calls_per_min`, harness, (body.calls / (body.windowMs / 60_000)) * 1000);
  } else if (kind === "rpc") {
    add(`ledger.rpc.${body.method}`, harness, body.totalMs);
    // Shown in the seconds columns as KB: value / 1000 prints kilobytes.
    add(`ledger.rpc_response_kb.${body.method}`, harness, (body.bytes / 1024) * 1000);
  } else if (kind === "harness") {
    for (const [name, at] of Object.entries(body.turn ?? {})) if (name.endsWith("Ms")) add(`ledger.harness.${name}`, harness, at);
  } else {
    add(`ledger.${kind}.total`, harness, body.totalMs);
    for (const [name, , took] of body.steps ?? []) add(`ledger.${kind}.${name}`, harness, took);
  }
}

// Spawn-to-ready, which the adapters print but nothing stores.
const logPath = join(dataDir, "bridged.log");
if (existsSync(logPath)) {
  const pattern = /spawn-to-ready harness=(\w+) boundary=(\w+) elapsed_ms=(\d+)/;
  for (const line of readFileSync(logPath, "utf8").split("\n")) {
    const match = pattern.exec(line);
    if (match && match[2] !== "process_spawned") add(`harness.spawn_to_ready(${match[2]})`, match[1], Number(match[3]));
  }
}

function percentile(sorted, p) {
  if (!sorted.length) return NaN;
  const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[rank];
}

const rows = [...samples.entries()].map(([key, values]) => {
  const [metric, harness] = key.split("|");
  const sorted = values.slice().sort((a, b) => a - b);
  return {
    metric, harness, n: sorted.length,
    p50: percentile(sorted, 50), p90: percentile(sorted, 90), p95: percentile(sorted, 95),
    p99: percentile(sorted, 99), max: sorted.at(-1),
  };
}).sort((a, b) => a.metric.localeCompare(b.metric) || (a.harness === "all" ? -1 : b.harness === "all" ? 1 : a.harness.localeCompare(b.harness)));

if (asJson) {
  console.log(JSON.stringify({ since, sessions: sessions.length, counters: Object.fromEntries(counters), rows }, null, 2));
} else {
  const sec = (value) => (Number.isFinite(value) ? (value / 1000).toFixed(2) : "-").padStart(8);
  console.log(`since ${since}, ${sessions.length} user chats; sends: ${[...counters].map(([k, v]) => `${k}=${v}`).join(" ")}`);
  console.log(`${"metric".padEnd(48)} ${"harness".padEnd(9)} ${"n".padStart(6)}      p50      p90      p95      p99      max  (seconds; *_kb rows are KB, *calls_per_min rows are calls)`);
  for (const row of rows) {
    console.log(`${row.metric.padEnd(48)} ${row.harness.padEnd(9)} ${String(row.n).padStart(6)} ${sec(row.p50)} ${sec(row.p90)} ${sec(row.p95)} ${sec(row.p99)} ${sec(row.max)}`);
  }
}
