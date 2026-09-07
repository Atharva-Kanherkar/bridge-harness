import assert from "node:assert/strict";
import test from "node:test";

import { buildOptions, permissionOptions, sdkEffort } from "../options.mjs";

const base = {
  sessionId: "11111111-1111-4111-8111-111111111111",
  model: "sonnet",
  cwd: "/tmp/bridge",
  instructions: "Follow the worker contract.",
  writeMode: "ReadOnly",
};

test("fresh queries use the adapter-allocated session id", () => {
  const options = buildOptions({ ...base, resume: false });
  assert.equal(options.sessionId, base.sessionId);
  assert.equal(options.resume, undefined);
});

test("resumed queries use resume without requesting a new session id", () => {
  const options = buildOptions({ ...base, resume: true });
  assert.equal(options.resume, base.sessionId);
  assert.equal(options.sessionId, undefined);
});

test("query options load explicit Claude plugins and credential-free connectors", () => {
  const mcpServers = { "claude.ai Notion": { type: "http", url: "https://mcp.example/notion" } };
  const options = buildOptions({ ...base, resume: false, plugins: ["/tmp/claude-plugins/notion"], mcpServers });
  assert.deepEqual(options.settingSources, ["project", "local"]);
  assert.equal(options.strictMcpConfig, false);
  assert.deepEqual(options.mcpServers, mcpServers);
  assert.deepEqual(options.plugins, [{ type: "local", path: "/tmp/claude-plugins/notion" }]);
  assert.deepEqual(options.systemPrompt, {
    type: "preset",
    preset: "claude_code",
    append: base.instructions,
  });
  assert.equal(options.permissionMode, "dontAsk");
  assert.deepEqual(options.allowedTools, ["Read", "Grep", "Glob", "Bash"]);
  assert.deepEqual(options.disallowedTools, ["Edit", "Write", "NotebookEdit"]);
});

test("Claude receives the compiled stable prefix before variable context", () => {
  const stable = '<bridge-stable-prompt schema="1">stable-provider-contract</bridge-stable-prompt>';
  const compile = evidence => [stable, `<bridge-variable-context>${evidence}</bridge-variable-context>`].join("\n\n");
  const first = buildOptions({ ...base, instructions: compile("variable-task-evidence-one"), resume: false }).systemPrompt.append;
  const second = buildOptions({ ...base, instructions: compile("variable-task-evidence-two"), resume: false }).systemPrompt.append;
  assert.notEqual(first, second);
  for (const appended of [first, second]) {
    assert.ok(appended.startsWith(stable));
    assert.ok(appended.indexOf("stable-provider-contract") < appended.indexOf("variable-task-evidence"));
  }
});

test("routed effort is passed to the SDK natively, low and medium included", () => {
  for (const level of ["low", "medium", "high", "xhigh", "max"]) {
    const options = buildOptions({ ...base, resume: false, effort: level });
    assert.equal(options.effort, level);
  }
});

test("effort is normalized and an out-of-set level is dropped rather than sent", () => {
  assert.equal(buildOptions({ ...base, resume: false, effort: "HIGH " }).effort, "high");
  // Codex's `ultra` has no Claude equivalent, so it is omitted entirely.
  assert.equal(buildOptions({ ...base, resume: false, effort: "ultra" }).effort, undefined);
  assert.equal(buildOptions({ ...base, resume: false, effort: null }).effort, undefined);
  assert.equal(sdkEffort("xhigh"), "xhigh");
  assert.equal(sdkEffort("ultra"), null);
  assert.equal(sdkEffort(undefined), null);
});

test("read-only mode denies direct write tools without dangerous bypass", () => {
  const options = permissionOptions("ReadOnly");
  assert.equal(options.permissionMode, "dontAsk");
  assert.equal(options.allowDangerouslySkipPermissions, undefined);
  assert.deepEqual(options.allowedTools, ["Read", "Grep", "Glob", "Bash"]);
  assert.deepEqual(options.disallowedTools, ["Edit", "Write", "NotebookEdit"]);
});

test("shared and isolated modes accept edits without bypassing permissions", () => {
  for (const mode of ["Shared", "Isolated"]) {
    const options = permissionOptions(mode);
    assert.deepEqual(options, { permissionMode: "acceptEdits" });
  }
});

test("full and default modes intentionally enable permission bypass", () => {
  for (const mode of ["Full", undefined, "unknown"]) {
    const options = permissionOptions(mode);
    assert.deepEqual(options, {
      permissionMode: "bypassPermissions",
      allowDangerouslySkipPermissions: true,
    });
  }
});


test("catalogue discovery does not start project integrations", async () => {
  const { catalogOptions } = await import("../options.mjs");
  assert.deepEqual(catalogOptions(), { settingSources: [], strictMcpConfig: true, mcpServers: {}, plugins: [], tools: [] });
});
