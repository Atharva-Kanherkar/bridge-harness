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

test("query options inherit Claude plugins and connectors without inline credentials", () => {
  const options = buildOptions({ ...base, resume: false });
  assert.deepEqual(options.settingSources, ["user", "project", "local"]);
  assert.equal(options.strictMcpConfig, false);
  assert.deepEqual(options.mcpServers, {});
  assert.deepEqual(options.systemPrompt, {
    type: "preset",
    preset: "claude_code",
    append: base.instructions,
  });
  assert.equal(options.permissionMode, "dontAsk");
  assert.deepEqual(options.allowedTools, ["Read", "Grep", "Glob", "Bash"]);
  assert.deepEqual(options.disallowedTools, ["Edit", "Write", "NotebookEdit"]);
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
