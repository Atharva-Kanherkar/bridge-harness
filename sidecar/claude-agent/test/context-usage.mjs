import { test } from "node:test";
import assert from "node:assert/strict";
import { contextUsageFrame, readContextUsage } from "../usage.mjs";

const response = {
  categories: [
    { name: "Messages", tokens: 61000, color: "x", kind: "used" },
    { name: "System tools", tokens: 14000, color: "x", kind: "used" },
    { name: "Free space", tokens: 900000, color: "x", kind: "free" },
    { name: "Autocompact buffer", tokens: 45000, color: "x", kind: "buffer" },
    { name: "Deferred tools", tokens: 3000, color: "x", isDeferred: true },
  ],
  totalTokens: 142000.4,
  maxTokens: 1000000,
  rawMaxTokens: 1000000,
  percentage: 14,
  gridRows: [],
  model: "claude-opus-5-5",
  memoryFiles: [{ path: "/a/CLAUDE.md", type: "Project", tokens: 2000 }, { path: "/b/CLAUDE.md", type: "User", tokens: 1000 }],
  mcpTools: [
    { name: "a", serverName: "railway", tokens: 4000 },
    { name: "b", serverName: "railway", tokens: 5000 },
    { name: "c", serverName: "notion", tokens: 2000 },
  ],
  agents: [{ agentType: "x", source: "y", tokens: 700 }],
  skills: { totalSkills: 73, includedSkills: 40, tokens: 6000, skillFrontmatter: [] },
  autoCompactThreshold: 955000,
  isAutoCompactEnabled: true,
  messageBreakdown: {
    toolCallTokens: 9000, toolResultTokens: 38000, attachmentTokens: 0, assistantMessageTokens: 8000,
    userMessageTokens: 6000, redirectedContextTokens: 0, unattributedTokens: 0,
    toolCallsByType: [
      { name: "Read", callTokens: 1000, resultTokens: 13000 },
      { name: "Bash", callTokens: 2000, resultTokens: 19000 },
    ],
    attachmentsByType: [],
  },
  apiUsage: null,
};

test("contextUsageFrame maps the SDK's /context data to Bridge's frame", () => {
  const frame = contextUsageFrame(response);
  assert.equal(frame.type, "context_usage");
  assert.equal(frame.model, "claude-opus-5-5");
  assert.equal(frame.usedTokens, 142000);
  assert.equal(frame.windowTokens, 1000000);
  assert.equal(frame.autoCompactTokens, 955000);
  assert.equal(frame.autoCompactEnabled, true);
  assert.deepEqual(frame.categories.map((c) => [c.name, c.kind]), [
    ["Free space", "free"], ["Messages", "used"], ["Autocompact buffer", "buffer"], ["System tools", "used"], ["Deferred tools", "deferred"],
  ]);
  assert.deepEqual(frame.memoryFiles, { count: 2, tokens: 3000 });
  assert.deepEqual(frame.skills, { included: 40, total: 73, tokens: 6000 });
  assert.equal(frame.agentsTokens, 700);
  assert.equal(frame.messages.toolResults, 38000);
  assert.deepEqual(frame.messages.toolsByType, [{ name: "Bash", tokens: 21000 }, { name: "Read", tokens: 14000 }]);
});

test("contextUsageFrame aggregates MCP tools by server, largest first", () => {
  assert.deepEqual(contextUsageFrame(response).mcpServers, [
    { name: "railway", tokens: 9000, tools: 2, loaded: 2, deferredTokens: 0 },
    { name: "notion", tokens: 2000, tools: 1, loaded: 1, deferredTokens: 0 },
  ]);
});

test("contextUsageFrame counts only loaded MCP tools as a per-turn cost", () => {
  // Tool search on: Notion's schemas are deferred until the model looks one up,
  // so the 85k the SDK reports for them is not in the window.
  const frame = contextUsageFrame({
    ...response,
    mcpTools: [
      ...Array.from({ length: 44 }, (_, index) => ({ name: `n${index}`, serverName: "claude_ai_Notion", tokens: 1943, isLoaded: false })),
      { name: "notion-search", serverName: "claude_ai_Notion", tokens: 1200, isLoaded: true },
      { name: "deploy", serverName: "railway", tokens: 800, isLoaded: true },
    ],
  });
  assert.deepEqual(frame.mcpServers, [
    { name: "claude_ai_Notion", tokens: 1200, tools: 45, loaded: 1, deferredTokens: 85492 },
    { name: "railway", tokens: 800, tools: 1, loaded: 1, deferredTokens: 0 },
  ]);
});

test("contextUsageFrame caps lists, strips control characters and truncates names", () => {
  const many = Array.from({ length: 40 }, (_, index) => ({ name: `c${index}\u0007`, tokens: index, kind: "used" }));
  const frame = contextUsageFrame({
    ...response,
    categories: [{ name: `${"x".repeat(200)}`, tokens: 1, kind: "used" }, ...many],
    mcpTools: Array.from({ length: 40 }, (_, index) => ({ name: "t", serverName: `s${index}`, tokens: index })),
    messageBreakdown: { ...response.messageBreakdown, toolCallsByType: Array.from({ length: 20 }, (_, index) => ({ name: `T${index}`, callTokens: index, resultTokens: 0 })) },
  });
  assert.equal(frame.categories.length, 16);
  assert.equal(frame.mcpServers.length, 16);
  assert.equal(frame.messages.toolsByType.length, 8);
  assert.ok(frame.categories.every((c) => !/[\x00-\x1f]/.test(c.name) && c.name.length <= 80));
});

test("contextUsageFrame rejects malformed input", () => {
  assert.equal(contextUsageFrame(null), null);
  assert.equal(contextUsageFrame({ ...response, totalTokens: undefined }), null);
  assert.equal(contextUsageFrame({ ...response, maxTokens: 0 }), null);
  assert.equal(contextUsageFrame({ ...response, totalTokens: -1 }), null);
  const bare = contextUsageFrame({ totalTokens: 10, maxTokens: 100 });
  assert.deepEqual(bare.categories, []);
  assert.equal(bare.messages, null);
  assert.equal(bare.skills, null);
  assert.equal(bare.autoCompactTokens, null);
});

test("readContextUsage is silent when the method is absent, throws, or times out", async () => {
  assert.equal(await readContextUsage({}), null);
  assert.equal(await readContextUsage(null), null);
  assert.equal(await readContextUsage({ getContextUsage: async () => { throw new Error("boom"); } }), null);
  assert.equal(await readContextUsage({ getContextUsage: () => new Promise(() => {}) }, 20), null);
  const frame = await readContextUsage({ getContextUsage: async () => response });
  assert.equal(frame.usedTokens, 142000);
});

test("the sidecar reports context usage after a result frame without blocking the pump", async (t) => {
  const { spawn } = await import("node:child_process");
  const { mkdtemp, rm, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { fileURLToPath } = await import("node:url");
  const root = await mkdtemp(join(tmpdir(), "bridge-sidecar-context-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sdk = join(root, "sdk.mjs");
  await writeFile(sdk, `
export function query() {
  let calls = 0;
  return {
    close() {},
    async getContextUsage() { calls += 1; return { totalTokens: 1200, maxTokens: 200000, categories: [], calls }; },
    async *[Symbol.asyncIterator]() {
      yield { type: "assistant", message: { content: [{ type: "text", text: "hi" }] } };
      yield { type: "result", subtype: "success", result: "done" };
      await new Promise((resolve) => setTimeout(resolve, 200));
    },
  };
}`);
  const child = spawn(process.execPath, [fileURLToPath(new URL("../index.mjs", import.meta.url)), "{}"], {
    env: { ...process.env, BRIDGE_CLAUDE_SDK_ENTRY: sdk }, stdio: ["pipe", "pipe", "pipe"],
  });
  let out = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => { out += chunk; });
  await new Promise((resolve) => child.once("close", resolve));
  const frames = out.trim().split("\n").map((line) => JSON.parse(line));
  assert.deepEqual(frames.map((frame) => frame.type), ["assistant", "result", "context_usage"]);
  assert.equal(frames[2].usedTokens, 1200);
  assert.equal(frames[2].windowTokens, 200000);
});
