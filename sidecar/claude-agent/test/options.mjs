import assert from "node:assert/strict";
import test from "node:test";

import { buildOptions, permissionOptions } from "../options.mjs";

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
  const instructions = [
    '<bridge-stable-prompt schema="1">stable-provider-contract</bridge-stable-prompt>',
    '<bridge-variable-context>variable-task-evidence</bridge-variable-context>',
  ].join("\n\n");
  const options = buildOptions({ ...base, instructions, resume: false });
  const appended = options.systemPrompt.append;
  assert.ok(appended.startsWith("<bridge-stable-prompt"));
  assert.ok(appended.indexOf("stable-provider-contract") < appended.indexOf("variable-task-evidence"));
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
