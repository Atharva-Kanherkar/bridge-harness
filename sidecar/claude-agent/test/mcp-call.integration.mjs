// The real sidecar and the real Agent SDK, against a local stdio MCP server.
// No network and no model: `mcp_call` runs the tool through Claude Code's own
// MCP client. Claude Code still has to start, so this runs only where it can
// (an authenticated runtime, or BRIDGE_SIDECAR_LIVE_TESTS=1 on a signed-in Mac).

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const runnable = Boolean(
  process.env.BRIDGE_SIDECAR_LIVE_TESTS === "1"
    || process.env.CLAUDE_CODE_OAUTH_TOKEN
    || process.env.ANTHROPIC_API_KEY,
);

test(
  "a direct call runs the approved tool once, through the harness, with no model turn",
  { skip: runnable ? false : "requires a Claude Code runtime that can start", timeout: 120_000 },
  async (t) => {
    const dir = mkdtempSync(join(tmpdir(), "bridge-mcp-call-"));
    t.after(() => rmSync(dir, { recursive: true, force: true }));
    const log = join(dir, "calls.log");
    const exactAction = { tool: "mcp__Echo_Local__echo", arguments: { text: "approved" } };
    const config = {
      sessionId: crypto.randomUUID(),
      model: "haiku",
      cwd: dir,
      // A key whose wire name differs from it, as a real connector's does. Not
      // `claude.ai ...`: a direct call drops explicit copies of those and lets
      // Claude Code load its own signed-in instance.
      mcpServers: {
        "Echo Local": { type: "stdio", command: process.execPath, args: [join(here, "fixtures/echo-mcp-server.mjs"), log] },
      },
      briefing: {
        allowedTools: [],
        allowedServers: [],
        readScopeServers: [],
        actionScope: { server: "Echo Local", intent: "post", permittedWords: ["send", "post", "reply"], forbiddenWords: ["delete"] },
        exactAction,
        directCall: true,
        deniedBuiltins: [],
        maxArgumentBytes: 65536,
      },
    };
    const child = spawn(process.execPath, [join(here, "../index.mjs"), JSON.stringify(config)], {
      cwd: dir,
      stdio: ["pipe", "pipe", "inherit"],
    });
    t.after(() => child.kill("SIGKILL"));

    const frames = [];
    const waiters = [];
    createInterface({ input: child.stdout }).on("line", (line) => {
      let frame;
      try { frame = JSON.parse(line); } catch { return; }
      frames.push(frame);
      for (const waiter of waiters.splice(0)) waiter();
    });
    const resultFor = async (id) => {
      for (;;) {
        const found = frames.find((frame) => frame.type === "mcp_call_result" && frame.id === id);
        if (found) return found;
        await new Promise((resolve) => waiters.push(resolve));
      }
    };
    const send = (frame) => child.stdin.write(JSON.stringify(frame) + "\n");

    send({ type: "mcp_call", id: "changed", tool: exactAction.tool, arguments: { text: "changed" } });
    const refused = await resultFor("changed");
    assert.equal(refused.ok, false);

    send({ type: "mcp_call", id: "approved", ...exactAction });
    const done = await resultFor("approved");
    assert.equal(done.ok, true, done.error);
    assert.notEqual(done.result.isError, true);
    assert.deepEqual(JSON.parse(done.result.content[0].text), { echoed: { text: "approved" } });

    // The server saw exactly the approved call and nothing else.
    assert.deepEqual(readFileSync(log, "utf8").trim().split("\n").map((line) => JSON.parse(line)), [{ text: "approved" }]);
    // No model ran: no assistant message and no turn result.
    assert.equal(frames.some((frame) => frame.type === "assistant" || frame.type === "result"), false);
  },
);
