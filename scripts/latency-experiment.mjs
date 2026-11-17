#!/usr/bin/env node
// Drive a bridged daemon the way the desktop app does and time every hop.
//
//   node scripts/latency-experiment.mjs state --data-dir <dir> [--n 5]
//   node scripts/latency-experiment.mjs chat  --data-dir <dir> --harness claude [--model m] [--effort e] [--n 3]
//
// `state` is read-only: it times `state/get_state`, the payload that
// `create_configured_chat` and `start_chat` also return on every new chat.
//
// `chat` replays the app's new-chat path (App.tsx `submitNewChatDraft` then
// `deliverPrompt`): resolve settings, create the chat, prepare the turn, cold
// start it, submit, then wait for the first output and the end of the turn.
// A second, warm send follows in the same chat. Each run archives its chat.
// It costs real model turns, so point it at a scratch data directory unless
// you mean to add chats to your own.
//
// What it cannot see is the webview: React work after a frame arrives is not
// here. Event delivery is measured as the gap between an event's `createdAt`
// (daemon clock) and its arrival (this process's clock, same machine).

import { createConnection } from "node:net";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { performance } from "node:perf_hooks";

const [mode = "state", ...rest] = process.argv.slice(2);
const flag = (name, fallback) => {
  const at = rest.indexOf(name);
  return at >= 0 ? rest[at + 1] : fallback;
};
const dataDir = flag("--data-dir");
if (!dataDir) throw new Error("--data-dir is required");
const runs = Number(flag("--n", mode === "state" ? 5 : 3));
const harness = flag("--harness", "claude");
const model = flag("--model", null);
const effort = flag("--effort", null);
const prompt = flag("--prompt", "Reply with the single word: ok");
const minor = Number(flag("--minor", 38));

class Client {
  constructor(socketPath) {
    this.socket = createConnection(socketPath);
    this.pending = new Map();
    this.listeners = [];
    this.nextId = 1;

    this.socket.on("data", (chunk) => this.onData(chunk));
    this.ready = new Promise((resolve, reject) => {
      this.socket.once("connect", resolve);
      this.socket.once("error", reject);
    });
  }
  onData(chunk) {
    // Collect chunks and join only at a newline: concatenating on every chunk
    // is quadratic in the frame, and a 36 MB state frame then measures this
    // reader instead of the daemon.
    this.chunks ??= [];
    let start = 0;
    let newline;
    while ((newline = chunk.indexOf(10, start)) >= 0) {
      this.chunks.push(chunk.subarray(start, newline));
      const line = Buffer.concat(this.chunks);
      this.chunks = [];
      start = newline + 1;
      this.deliver(line);
    }
    if (start < chunk.length) this.chunks.push(chunk.subarray(start));
  }
  deliver(line) {
    {
      const receivedAt = performance.now();
      const wall = Date.now();
      const message = JSON.parse(line.toString("utf8"));
      if (message.id !== undefined && this.pending.has(message.id)) {
        const { resolve, reject, sentAt } = this.pending.get(message.id);
        this.pending.delete(message.id);
        const timing = { ms: receivedAt - sentAt, bytes: line.length };
        if (message.error) reject(Object.assign(new Error(message.error.message), { rpc: message.error, timing }));
        else resolve({ result: message.result, timing });
      } else if (message.method) {
        for (const listener of this.listeners) listener(message, receivedAt, wall);
      }
    }
  }
  call(method, params) {
    const id = this.nextId++;
    const frame = { jsonrpc: "2.0", id, method, ...(params === undefined ? {} : { params }) };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, sentAt: performance.now() });
      this.socket.write(JSON.stringify(frame) + "\n");
    });
  }
  close() { this.socket.end(); }
}

async function connect() {
  const client = new Client(join(dataDir, "bridged.sock"));
  await client.ready;
  const authToken = readFileSync(join(dataDir, "daemon.token"), "utf8").trim();
  await client.call("protocol/handshake", {
    protocolVersion: { major: 1, minor },
    client: { name: "latency-experiment", version: "0" },
    authToken,
  });
  return client;
}

const stats = (values) => {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  const at = (p) => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))];
  return sorted.length ? { n: sorted.length, p50: at(0.5), p95: at(0.95), max: sorted.at(-1) } : { n: 0 };
};

async function stateMode() {
  const client = await connect();
  const samples = [];
  for (let run = 0; run < runs; run++) {
    const { result, timing } = await client.call("state/get_state");
    const parseStarted = performance.now();
    JSON.parse(JSON.stringify(result));
    samples.push({ ms: timing.ms, bytes: timing.bytes, sessions: result.sessions?.length, reparseMs: performance.now() - parseStarted });
  }
  client.close();
  console.log(JSON.stringify({ mode: "state", dataDir, samples, rpc: stats(samples.map((s) => s.ms)) }, null, 2));
}

const OUTPUT_KINDS = new Set([
  "message.delta", "reasoning.delta", "reasoning.started", "message.started",
  "tool.started", "command.started", "file_change.started", "message.completed",
]);

/** One send: prepare, optionally cold start, submit, then wait for output. */
async function send(client, sessionId, text, { cold }) {
  const marks = {};
  const t0 = performance.now();
  const mark = (name) => { marks[name] ??= Math.round(performance.now() - t0); };
  const kinds = {};
  const deliveryLag = [];
  // Present when the daemon runs with BRIDGE_STREAM_TIMING=1.
  const frame = { dbWaitMs: [], persistenceMs: [], receiptToPublicationMs: [] };
  let done;
  const finished = new Promise((resolve) => { done = resolve; });
  const listener = (message, receivedAt, wall) => {
    if (message.method === "session-startup" && message.params?.sessionId === sessionId) {
      mark(`startup.${message.params.phase}`);
    }
    if (message.method !== "agent-event") return;
    const event = message.params;
    if (event?.sessionId !== sessionId) return;
    const at = Math.round(receivedAt - t0);
    kinds[event.kind] ??= at;
    if (event.createdAt) deliveryLag.push(wall - Date.parse(event.createdAt));
    const streamTiming = event.providerMeta?.bridgeStreamTiming;
    if (streamTiming) for (const key of Object.keys(frame)) frame[key].push(streamTiming[key]);
    if (event.kind === "message.completed" && event.role === "user") mark("event.user_message");
    if (event.kind === "turn.started") mark("event.turn_started");
    if (OUTPUT_KINDS.has(event.kind) && event.role !== "user") mark("event.first_output");
    if (event.kind === "message.delta") mark("event.first_text");
    if (event.kind === "turn.completed") { mark("event.turn_completed"); done(); }
  };
  client.listeners.push(listener);
  const rpc = {};
  const timed = async (name, method, params) => {
    const { result, timing } = await client.call(method, params);
    rpc[name] = { ms: Math.round(timing.ms), bytes: timing.bytes };
    mark(`rpc.${name}`);
    return result;
  };
  await timed("prepare_turn", "sessions/prepare_turn", { sessionId, text });
  if (cold) await timed("start_chat", "sessions/start_chat", { sessionId });
  await timed("submit_input", "sessions/submit_input", { sessionId, text });
  const timeout = setTimeout(done, 180_000);
  await finished;
  clearTimeout(timeout);
  client.listeners.splice(client.listeners.indexOf(listener), 1);
  return {
    marks, rpc, firstSeenByKind: kinds, deliveryLagMs: stats(deliveryLag),
    perFrame: Object.fromEntries(Object.entries(frame).map(([key, values]) => [key, stats(values)])),
  };
}

async function chatMode() {
  const client = await connect();
  const results = [];
  for (let run = 0; run < runs; run++) {
    const overrides = {
      target: { kind: "direct" },
      model: { harness, model },
      effort: { value: effort },
    };
    const t0 = performance.now();
    const settings = await client.call("sessions/resolve_chat_settings", { workspaceId: null, overrides, expectedRevision: null });
    const created = await client.call("sessions/create_configured_chat", {
      workspaceId: null, overrides, expectedRevision: settings.result.revision, title: null,
    });
    const sessionId = created.result.sessionId;
    const createMs = Math.round(performance.now() - t0);
    const first = await send(client, sessionId, prompt, { cold: true });
    const warm = await send(client, sessionId, prompt, { cold: false });
    results.push({
      run, sessionId,
      newChat: {
        resolve_chat_settings: { ms: Math.round(settings.timing.ms), bytes: settings.timing.bytes },
        create_configured_chat: { ms: Math.round(created.timing.ms), bytes: created.timing.bytes },
        clickToCreated: createMs,
        firstSend: first,
        clickToFirstOutput: createMs + (first.marks["event.first_output"] ?? NaN),
      },
      warmSend: warm,
    });
    await client.call("sessions/archive_chat", { sessionId }).catch(() => undefined);
    console.error(`run ${run}: click->first output ${results.at(-1).newChat.clickToFirstOutput} ms, warm ${warm.marks["event.first_output"]} ms`);
  }
  client.close();
  console.log(JSON.stringify({ mode: "chat", dataDir, harness, model, effort, results }, null, 2));
}

await (mode === "state" ? stateMode() : chatMode());
