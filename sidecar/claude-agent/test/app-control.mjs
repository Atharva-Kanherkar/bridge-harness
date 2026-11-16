import { test } from "node:test";
import assert from "node:assert/strict";
import { createAppControl } from "../app-control.mjs";

function setup(overrides = {}) {
  const replies = [], calls = [];
  const server = { name: "claude.ai Slack", status: "connected", config: { password: "hidden" }, tools: [
    { name: "slack_search_users", description: "hidden description", annotations: { readOnly: true } },
    { name: "slack_send_message" },
    { name: "get_and_delete", annotations: { readOnly: true } },
    { name: "read_unsafe", annotations: { destructive: true } },
    { name: "read_write", annotations: { readOnly: false } },
    { name: "get_writable", annotations: { readOnly: false } },
  ] };
  const handler = createAppControl({ enabled: true, status: async () => [server], request: async input => { calls.push(input); return { response: { content: [{ type: "text", text: "found" }] } }; }, writeFrame: frame => replies.push(frame), ...overrides });
  const send = async request => { await handler({ type: "app_control", id: "id", request }); return replies.at(-1); };
  return { send, handler, replies, calls, server };
}
const read = tool => ({ kind: "read", server: "claude.ai Slack", tool, arguments: { query: "a" } });

test("inventory selects and sanitizes the existing query's server", async () => {
  const { send } = setup();
  const result = await send({ kind: "inventory", server: "claude_ai_Slack" });
  assert.equal(result.ok, true);
  assert.equal(result.result.name, "claude.ai Slack");
  assert.equal(JSON.stringify(result).includes("hidden"), false);
  assert.deepEqual(Object.keys(result.result.tools[0]), ["name", "readOnly", "destructive"]);
});
test("disabled launches, disconnected servers and ambiguous wire aliases refuse", async () => {
  assert.equal((await setup({ enabled: false }).send(read("slack_search_users"))).ok, false);
  for (const servers of [[{ name: "claude.ai Slack", status: "failed" }], [{ name: "claude.ai Slack", status: "connected" }, { name: "claude_ai_Slack", status: "connected" }]]) {
    assert.equal((await setup({ status: async () => servers }).send(read("slack_search_users"))).ok, false);
  }
});
test("only safe real reads reach MCP, without model context", async () => {
  const { send, calls } = setup();
  for (const tool of ["slack_send_message", "get_and_delete", "read_unsafe", "get_writable", "missing_get", "SLACK_search_users"]) {
    assert.equal((await send(read(tool))).ok, false, tool);
  }
  assert.equal(calls.length, 0);
  assert.equal((await send(read("slack_search_users"))).ok, true);
  assert.deepEqual(calls[0], { subtype: "mcp_call", tool: "mcp__claude_ai_Slack__slack_search_users", arguments: { query: "a" }, check_permissions: false, result_not_for_model: true });
});
test("arguments, results, unsupported operations and error text are bounded", async () => {
  const { send, calls } = setup();
  assert.equal((await send({ ...read("slack_search_users"), kind: "write" })).ok, false);
  for (const args of [[], null, { query: "x".repeat(65536) }]) assert.equal((await send({ ...read("slack_search_users"), arguments: args })).ok, false);
  assert.equal(calls.length, 0);
  const large = setup({ request: async () => ({ response: { text: "x".repeat(262144) } }) });
  assert.equal((await large.send(read("slack_search_users"))).ok, false);
  const broken = setup({ status: async () => { throw new Error("secret config"); } });
  const failed = await broken.send(read("slack_search_users"));
  assert.equal(failed.ok, false);
  assert.equal(JSON.stringify(failed).includes("secret config"), false);
});
test("concurrent requests are capped and released after errors", async () => {
  let release;
  const status = new Promise(resolve => { release = resolve; });
  const { handler, replies } = setup({ status: () => status });
  const requests = Array.from({ length: 4 }, (_, i) => handler({ id: `r${i}`, request: read("slack_search_users") }));
  await handler({ id: "extra", request: read("slack_search_users") });
  assert.equal(replies[0].ok, false);
  release([]);
  await Promise.all(requests);
  assert.equal(replies.length, 5);
});
