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
//   { sessionId, model, cwd, resume, instructions, writeMode, plugins, mcpServers }

import { createInterface } from "node:readline";
import { pathToFileURL } from "node:url";
import { buildOptions, catalogOptions } from "./options.mjs";
import { userContentBlocks } from "./input.mjs";
import { probeUsage } from "./usage.mjs";

// Which copy of the Agent SDK to load.
//
// With a Bridge-managed payload installed, the Rust side sets
// BRIDGE_CLAUDE_SDK_ENTRY to that installation's `sdk.mjs`. Unset — the default,
// and the case for every existing install — this resolves the bundled dependency
// exactly as it did before. ESM ignores NODE_PATH, so an explicit module entry is
// the only way to redirect a bare specifier.
const sdkEntry = process.env.BRIDGE_CLAUDE_SDK_ENTRY;
const { query } = sdkEntry
  ? await import(pathToFileURL(sdkEntry).href)
  : await import("@anthropic-ai/claude-agent-sdk");

// A closed pipe means the parent can no longer receive protocol frames. Exit
// unsuccessfully instead of trying to report another frame to that same pipe.
process.stdout.on("error", () => process.exit(1));

function writeFrame(frame) {
  // Await the write callback, not just write()'s return value: even a write
  // below the high-water mark may still be buffered when process.exit runs.
  return new Promise((resolve, reject) => {
    process.stdout.write(JSON.stringify(frame) + "\n", (error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

async function fail(message) {
  try {
    await writeFrame({ type: "result", subtype: "error_sidecar", is_error: true, result: message });
  } finally {
    process.exit(1);
  }
}

let config;
try {
  config = JSON.parse(process.argv[2] ?? process.env.BRIDGE_CLAUDE_CONFIG ?? "{}");
} catch (error) {
  await fail(`Invalid sidecar config: ${error?.message ?? error}`);
}

const { sessionId } = config;

if (config.usage === true) {
  const frame = await probeUsage(query, config);
  await writeFrame(frame);
  process.exit(frame.type === "claude_usage" ? 0 : 1);
}

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

function userMessage(content) {
  return {
    type: "user",
    message: { role: "user", content },
    parent_tool_use_id: null,
    ...(sessionId ? { session_id: sessionId } : {}),
  };
}

const options = config.catalog === true ? catalogOptions() : buildOptions(config);

const run = query({ prompt: input, options });

// Catalogue discovery is a short-lived control-plane request. It uses the
// installed SDK/CLI itself, so new provider releases appear without a Bridge
// code change. No user turn is submitted and the normal stream pump is skipped.
if (config.catalog === true) {
  try {
    const models = await run.supportedModels();
    await writeFrame({ type: "model_catalog", models });
    run.close();
    process.exit(0);
  } catch (error) {
    await fail(`Claude model catalogue error: ${error?.message ?? error}`);
  }
}

// Control frames from Rust.
const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let frame;
  try { frame = JSON.parse(trimmed); } catch { return; }
  if (frame.type === "user") {
    // Forward content blocks as-is: image attachments arrive as
    // `[{type:"image",source:{...}}, …]` beside the text block. Flattening
    // here would silently drop them — the exact failure image paste exists
    // to remove. See input.mjs for the shapes this tolerates.
    const blocks = userContentBlocks(frame);
    if (blocks) input.push(userMessage(blocks));
  } else if (frame.type === "control_request" && frame?.request?.subtype === "interrupt") {
    void run.interrupt().catch(() => {});
  }
});
rl.on("close", () => input.close());

// Pump SDK messages straight to stdout as newline JSON.
try {
  for await (const message of run) {
    await writeFrame(message);
  }
} catch (error) {
  await fail(`Claude Agent SDK error: ${error?.message ?? error}`);
}
process.exit(0);
