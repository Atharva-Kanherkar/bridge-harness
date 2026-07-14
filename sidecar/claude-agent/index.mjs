#!/usr/bin/env node
// Bridge ↔ Claude Agent SDK sidecar.
//
// Replaces the per-turn `claude -p` spawn. One long-lived process drives a
// single streaming-input `query()`, so multiple turns share one session (fixing
// the `-p` "exit after one turn" problem). It speaks the same newline-delimited
// wire protocol the Rust adapter already used with `claude -p`:
//
//   stdin  (Rust → sidecar): control frames
//     {"type":"user","message":{"role":"user","content":[{"type":"text","text":"…"}]}}
//     {"type":"control_request","request":{"subtype":"interrupt"}}
//   stdout (sidecar → Rust): raw SDK messages (system/assistant/stream_event/
//     user/result), one JSON object per line — the exact shapes Bridge's
//     normalize_claude_message_with_state already understands.
//
// Config arrives as a single JSON argument (argv[2]):
//   { sessionId, model, cwd, resume, instructions, writeMode }

import { createInterface } from "node:readline";
import { query } from "@anthropic-ai/claude-agent-sdk";
import { buildOptions } from "./options.mjs";

function fail(message) {
  process.stdout.write(JSON.stringify({ type: "result", subtype: "error_sidecar", is_error: true, result: message }) + "\n");
  process.exit(1);
}

let config;
try {
  config = JSON.parse(process.argv[2] ?? process.env.BRIDGE_CLAUDE_CONFIG ?? "{}");
} catch (error) {
  fail(`Invalid sidecar config: ${error?.message ?? error}`);
}

const { sessionId } = config;

// Push-driven async iterable of SDKUserMessage: turns arrive on stdin over the
// life of the process and are fed into the one streaming query.
function makeInputStream() {
  const queue = [];
  let wake = null;
  let closed = false;
  return {
    push(message) {
      queue.push(message);
      if (wake) { wake(); wake = null; }
    },
    close() {
      closed = true;
      if (wake) { wake(); wake = null; }
    },
    async *[Symbol.asyncIterator]() {
      while (true) {
        while (queue.length) yield queue.shift();
        if (closed) return;
        await new Promise((resolve) => { wake = resolve; });
      }
    },
  };
}

const input = makeInputStream();

function userMessage(text) {
  return {
    type: "user",
    message: { role: "user", content: [{ type: "text", text }] },
    parent_tool_use_id: null,
    ...(sessionId ? { session_id: sessionId } : {}),
  };
}

const options = buildOptions(config);

const run = query({ prompt: input, options });

// Control frames from Rust.
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let frame;
  try { frame = JSON.parse(trimmed); } catch { return; }
  if (frame.type === "user") {
    const text = frame?.message?.content?.map?.((part) => part?.text ?? "").join("") ?? "";
    if (text) input.push(userMessage(text));
  } else if (frame.type === "control_request" && frame?.request?.subtype === "interrupt") {
    void run.interrupt().catch(() => {});
  }
});
rl.on("close", () => input.close());

// Pump SDK messages straight to stdout as newline JSON.
try {
  for await (const message of run) {
    process.stdout.write(JSON.stringify(message) + "\n");
  }
} catch (error) {
  fail(`Claude Agent SDK error: ${error?.message ?? error}`);
}
process.exit(0);
