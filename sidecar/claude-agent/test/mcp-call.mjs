import assert from "node:assert/strict";
import test from "node:test";

import { createDirectCall } from "../mcp-call.mjs";

const exactAction = {
  tool: "mcp__claude_ai_Slack__slack_send_message",
  arguments: { channel_id: "C123", message: "approved verbatim" },
};

function harness({ approved = exactAction, reply } = {}) {
  const requests = [];
  const frames = [];
  const handle = createDirectCall({
    exactAction: approved,
    request: async (request) => {
      requests.push(request);
      if (reply instanceof Error) throw reply;
      return { response: reply ?? { content: [{ type: "text", text: "{\"ok\":true}" }] } };
    },
    writeFrame: async (frame) => { frames.push(frame); },
  });
  return { handle, requests, frames };
}

test("the approved call is forwarded once, with the gate and raw results requested", async () => {
  const { handle, requests, frames } = harness();
  await handle({ type: "mcp_call", id: "c1", ...exactAction });
  assert.deepEqual(requests, [{
    subtype: "mcp_call",
    tool: exactAction.tool,
    arguments: exactAction.arguments,
    check_permissions: true,
    result_not_for_model: true,
  }]);
  assert.deepEqual(frames, [{
    type: "mcp_call_result", id: "c1", ok: true,
    result: { content: [{ type: "text", text: "{\"ok\":true}" }] },
  }]);
});

test("anything but the approved call never reaches the harness", async () => {
  for (const frame of [
    { id: "x", ...exactAction, tool: "mcp__claude_ai_Slack__slack_delete_message" },
    { id: "x", ...exactAction, arguments: { ...exactAction.arguments, message: "changed" } },
    { id: "x", ...exactAction, arguments: { ...exactAction.arguments, reply_broadcast: true } },
    { id: "x", tool: exactAction.tool },
    { ...exactAction },
    { id: "", ...exactAction },
  ]) {
    const { handle, requests, frames } = harness();
    await handle({ type: "mcp_call", ...frame });
    assert.equal(requests.length, 0, JSON.stringify(frame));
    assert.equal(frames.length, 1);
    assert.equal(frames[0].ok, false);
    assert.equal(frames[0].type, "mcp_call_result");
  }
});

test("a launch with no approved call refuses every direct call", async () => {
  const { handle, requests, frames } = harness({ approved: null });
  await handle({ type: "mcp_call", id: "c1", ...exactAction });
  assert.equal(requests.length, 0);
  assert.deepEqual(frames, [{ type: "mcp_call_result", id: "c1", ok: false, error: "this launch carries no approved call" }]);
});

test("the approval is consumed by the first call", async () => {
  const { handle, requests, frames } = harness();
  await handle({ type: "mcp_call", id: "c1", ...exactAction });
  await handle({ type: "mcp_call", id: "c2", ...exactAction });
  assert.equal(requests.length, 1);
  assert.equal(frames[1].ok, false);
  assert.equal(frames[1].error, "the approved call was already used");
});

test("a refusal from the harness is reported, not retried", async () => {
  const { handle, requests, frames } = harness({ reply: new Error("The tool did not run: denied") });
  await handle({ type: "mcp_call", id: "c1", ...exactAction });
  await handle({ type: "mcp_call", id: "c2", ...exactAction });
  assert.equal(requests.length, 1);
  assert.deepEqual(frames[0], { type: "mcp_call_result", id: "c1", ok: false, error: "The tool did not run: denied" });
});

test("a tool error comes back as a result for Rust to judge", async () => {
  const reply = { content: [{ type: "text", text: "channel_not_found" }], isError: true };
  const { handle, frames } = harness({ reply });
  await handle({ type: "mcp_call", id: "c1", ...exactAction });
  assert.deepEqual(frames, [{ type: "mcp_call_result", id: "c1", ok: true, result: reply }]);
});
