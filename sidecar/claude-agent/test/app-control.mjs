import { test } from "node:test";
import assert from "node:assert/strict";
import { createAppControl } from "../app-control.mjs";
import { connectorReadPolicy } from "../options.mjs";

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
  const handler = createAppControl({ enabled: true, status: async () => [server], request: async input => { calls.push(input); return { response: { content: [{ type: "text", text: "found" }] } }; }, writeFrame: frame => replies.push(frame), mayRead: () => true, ...overrides });
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
test("a chat whose mode refuses the model connector reads refuses preview reads too", async () => {
  for (const config of [{ writeMode: "Shared" }, { writeMode: "Isolated" }, { writeMode: "ReadOnly", networkAllowed: false, mcpServers: { claude_ai_Slack: {} } }, { writeMode: "ReadOnly", networkAllowed: true, mcpServers: { bridge: {} } }]) {
    const { send, calls } = setup({ mayRead: connectorReadPolicy(config) });
    const reply = await send(read("slack_search_users"));
    assert.equal(reply.ok, false, JSON.stringify(config));
    assert.match(reply.error, /access mode/);
    assert.equal(calls.length, 0);
    assert.equal((await send({ kind: "inventory", server: "claude.ai Slack" })).ok, true, "inventory calls no tool");
  }
  for (const config of [{ writeMode: "Full" }, {}, { writeMode: "ReadOnly", networkAllowed: true, mcpServers: { claude_ai_Slack: {} } }]) {
    const { send, calls } = setup({ mayRead: connectorReadPolicy(config) });
    assert.equal((await send(read("slack_search_users"))).ok, true, JSON.stringify(config));
    assert.equal(calls.length, 1);
  }
});
test("a launch without a read policy refuses reads", async () => {
  const { send, calls } = setup({ mayRead: undefined });
  assert.equal((await send(read("slack_search_users"))).ok, false);
  assert.equal(calls.length, 0);
});
test("an app runtime serves its one server: inventory and reads elsewhere refuse", async () => {
  const serves = name => name === "claude.ai Slack";
  const other = { name: "claude.ai Linear", status: "connected", tools: [{ name: "linear_list_issues", annotations: { readOnly: true } }] };
  const { send, calls } = setup({ mayRead: serves, serverAllowed: serves, status: async () => [setup().server, other] });
  assert.equal((await send({ kind: "inventory", server: "claude.ai Slack" })).ok, true);
  assert.equal((await send({ kind: "inventory", server: "claude.ai Linear" })).ok, false);
  assert.equal((await send({ kind: "read", server: "claude.ai Linear", tool: "linear_list_issues", arguments: {} })).ok, false);
  assert.equal(calls.length, 0);
  assert.equal((await send(read("slack_search_users"))).ok, true);
  assert.equal((await send({ ...read("slack_send_message") })).ok, false, "never a write");
});
test("a server not yet started is asked for once, then listed", async () => {
  const listed = setup().server;
  let started = false;
  const calls = [];
  const { send } = setup({
    status: async () => (started ? [listed] : []),
    request: async input => { calls.push(input.subtype); if (input.subtype === "mcp_reconnect") started = true; return { response: { content: [] } }; },
  });
  assert.equal((await send({ kind: "inventory", server: "claude.ai Slack" })).ok, true);
  assert.deepEqual(calls, ["mcp_reconnect"]);
  const never = setup({ status: async () => [], serverAllowed: () => false, request: async input => { calls.push(input.subtype); return {}; } });
  assert.equal((await never.send({ kind: "inventory", server: "claude.ai Slack" })).ok, false);
  assert.deepEqual(calls, ["mcp_reconnect"], "a server this launch does not serve is never started");
});
test("servers lists names and statuses only, even on a launch that serves one server", async () => {
  const { send, calls } = setup({ serverAllowed: name => name === "claude.ai Slack", status: async () => [
    { name: "claude.ai Slack", status: "connected", config: { password: "hidden" }, tools: [{ name: "slack_search_users" }] },
    { name: "openaiDeveloperDocs", status: "pending", tools: [] },
  ] });
  const result = await send({ kind: "servers" });
  assert.equal(result.ok, true);
  assert.deepEqual(result.result, { servers: [{ name: "claude.ai Slack", status: "connected" }, { name: "openaiDeveloperDocs", status: "pending" }] });
  assert.equal(JSON.stringify(result).includes("hidden") || JSON.stringify(result).includes("slack_search_users"), false, "no config or tools");
  assert.deepEqual(calls, [], "no MCP call is made");
});
