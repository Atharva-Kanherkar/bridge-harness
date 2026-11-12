import test from "node:test";
import assert from "node:assert/strict";
import { makeWorkflowGate, briefingOptions } from "../briefing.mjs";
import { createAskRelay } from "../ask.mjs";

const servers = [
  { key: "claude.ai Slack", wire: "claude_ai_Slack", read: true, write: "ask" },
  { key: "claude.ai Linear", wire: "claude_ai_Linear", read: true, write: "never" },
];

test("the workflow gate allows a read, asks for a write, and denies everything else", async () => {
  const asked = [];
  const gate = makeWorkflowGate({
    servers,
    ask: async (tool) => { asked.push(tool); return tool.endsWith("send_message") ? "allow" : "deny"; },
  });
  assert.equal((await gate("mcp__claude_ai_Slack__slack_read_thread", {}, { toolUseID: "t1" })).behavior, "allow");
  assert.equal((await gate("mcp__claude_ai_Linear__list_issues", {})).behavior, "allow");
  assert.deepEqual(asked, [], "reads never ask");

  assert.equal((await gate("mcp__claude_ai_Slack__slack_send_message", { text: "hi" })).behavior, "allow");
  const declined = await gate("mcp__claude_ai_Slack__slack_delete_message", {});
  assert.equal(declined.behavior, "deny");
  assert.match(declined.message, /slack_delete_message/);
  assert.deepEqual(asked, ["mcp__claude_ai_Slack__slack_send_message", "mcp__claude_ai_Slack__slack_delete_message"]);

  // A write on a read-only server, an ungranted server, and a built-in are denied by name.
  for (const tool of ["mcp__claude_ai_Linear__create_issue", "mcp__claude_ai_Gmail__send_email", "Bash"]) {
    const denied = await gate(tool, {});
    assert.equal(denied.behavior, "deny", tool);
    assert.ok(denied.message.includes(tool), denied.message);
  }
  assert.equal(asked.length, 2, "denied calls never reach Bridge");
});

test("a read verb beside a mutation word is a write and asks", async () => {
  const asked = [];
  const gate = makeWorkflowGate({ servers, ask: async (tool) => { asked.push(tool); return "deny"; } });
  assert.equal((await gate("mcp__claude_ai_Slack__get_or_create_channel", {})).behavior, "deny");
  assert.deepEqual(asked, ["mcp__claude_ai_Slack__get_or_create_channel"]);
});

test("workflow options never pre-approve, stay strict, and reach only granted servers", () => {
  const options = briefingOptions(
    { workflow: { servers }, deniedBuiltins: ["Bash", "Write"], maxArgumentBytes: 4096 },
    { "claude.ai Slack": { type: "http" }, "claude.ai Linear": { type: "http" }, "claude.ai Gmail": { type: "http" } },
    async () => "deny",
  );
  assert.equal(options.allowedTools, undefined);
  assert.equal(options.strictMcpConfig, true);
  assert.deepEqual(Object.keys(options.mcpServers).sort(), ["claude.ai Linear", "claude.ai Slack"]);
  assert.deepEqual(options.disallowedTools, ["Bash", "Write"]);
  assert.deepEqual(options.settingSources, []);
  assert.equal(typeof options.canUseTool, "function");
});

test("the ask relay matches answers by id and denies what is pending when the session ends", async () => {
  const frames = [];
  const relay = createAskRelay(async (frame) => { frames.push(frame); });
  const first = relay.ask("mcp__claude_ai_Slack__slack_send_message", { text: "a" }, "tool-1");
  const second = relay.ask("mcp__claude_ai_Slack__slack_send_message", { text: "b" }, "tool-2");
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(frames.length, 2);
  assert.equal(frames[0].type, "control_request");
  assert.equal(frames[0].request.subtype, "can_use_tool");
  assert.deepEqual(frames[0].request.tool_input, { text: "a" });
  assert.equal(frames[0].request.tool_use_id, "tool-1");

  assert.equal(relay.resolve({ type: "control_response", response: { subtype: "success", request_id: frames[1].request_id, response: { behavior: "allow" } } }), true);
  assert.equal(await second, "allow");
  assert.equal(relay.resolve({ type: "control_response", response: { request_id: "wf-unknown", response: { behavior: "allow" } } }), false);

  relay.close();
  assert.equal(await first, "deny");
  assert.equal(await relay.ask("mcp__claude_ai_Slack__slack_send_message", {}), "deny", "nothing is asked after the session ends");
  assert.equal(frames.length, 2);
});

test("an interrupt denies what is waiting but later asks still go out", async () => {
  const frames = [];
  const relay = createAskRelay(async (frame) => { frames.push(frame); });
  const waiting = relay.ask("mcp__claude_ai_Slack__slack_send_message", {});
  relay.denyPending();
  assert.equal(await waiting, "deny");
  const later = relay.ask("mcp__claude_ai_Slack__slack_send_message", {});
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(relay.pendingCount, 1);
  relay.resolve({ type: "control_response", response: { request_id: frames.at(-1).request_id, response: { behavior: "allow" } } });
  assert.equal(await later, "allow");
});
