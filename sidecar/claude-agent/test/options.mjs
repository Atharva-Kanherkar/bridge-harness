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
  const options = buildOptions({ ...base, writeMode: "Shared", resume: false, plugins: ["/tmp/claude-plugins/notion"], mcpServers, projectTrusted: true });
  assert.deepEqual(options.settingSources, ["user", "project", "local"]);
  assert.equal(options.skills, "all");
  assert.equal(options.strictMcpConfig, false);
  assert.deepEqual(options.mcpServers, mcpServers);
  assert.deepEqual(options.plugins, [{ type: "local", path: "/tmp/claude-plugins/notion", skipMcpDiscovery: true }]);
  assert.deepEqual(options.systemPrompt, {
    type: "preset",
    preset: "claude_code",
    append: base.instructions,
  });
  assert.equal(options.permissionMode, "acceptEdits");
  assert.equal(options.permissionPrompts, undefined);
  assert.equal(options.allowedTools, undefined);
  assert.equal(options.tools, undefined);
  assert.equal(options.disallowedTools, undefined);
});

test("a folder's own settings load only when the folder is trusted", () => {
  for (const writeMode of ["Shared", "Isolated", "Full", undefined]) {
    assert.deepEqual(buildOptions({ ...base, writeMode }).settingSources, ["user"], `${writeMode} defaults to untrusted`);
    assert.deepEqual(buildOptions({ ...base, writeMode, projectTrusted: false }).settingSources, ["user"]);
    assert.deepEqual(buildOptions({ ...base, writeMode, projectTrusted: true }).settingSources, ["user", "project", "local"]);
  }
  // A read-only worker never loads them, trusted or not.
  assert.deepEqual(buildOptions({ ...base, writeMode: "ReadOnly", projectTrusted: true }).settingSources, ["user"]);
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
  assert.equal(options.permissionMode, "default");
  assert.equal(options.allowDangerouslySkipPermissions, undefined);
  assert.equal(options.allowedTools, undefined);
  assert.ok(options.tools.includes("Skill"));
  assert.deepEqual(options.disallowedTools, ["Edit", "Write", "NotebookEdit", "Task"]);
  assert.equal(typeof options.hooks.PreToolUse[0].hooks[0], "function");
});

test("read-only network tools follow the routed network authority", () => {
  assert.ok(!permissionOptions("ReadOnly", false).tools.includes("WebFetch"));
  assert.ok(permissionOptions("ReadOnly", true).tools.includes("WebFetch"));
  assert.ok(permissionOptions("ReadOnly", true).tools.includes("WebSearch"));
  assert.ok(permissionOptions("ReadOnly", true).tools.includes("Bash"));
});

test("an explicit plugin launch policy is preserved", () => {
  const options = buildOptions({
    ...base,
    writeMode: "Shared",
    plugins: [{ path: "/tmp/healthy", skipMcpDiscovery: false }],
  });
  assert.deepEqual(options.plugins, [{ type: "local", path: "/tmp/healthy", skipMcpDiscovery: false }]);
});

test("shared and isolated modes accept edits without bypassing permissions", () => {
  for (const mode of ["Shared", "Isolated"]) {
    const options = permissionOptions(mode);
    assert.deepEqual(options, { permissionMode: "acceptEdits" });
  }
});

test("ask mode runs Bridge's read-only tools without a prompt Claude cannot show", () => {
  // Bridge renders no Claude permission prompt, so a tool that needs one is
  // refused outright. Bridge's own read-only tools change nothing and run, a
  // memory proposal waits for the person's review in Bridge, a drafted message
  // only becomes a card the person sends themselves, a settings change cannot
  // widen authority and is undone the same way, and notes writes stay refused
  // in Ask mode.
  const bridge = { bridge: { type: "stdio", command: "/bin/bridged", args: ["--bridge-mcp-visualize"] } };
  for (const mode of ["Shared", "Isolated"]) {
    const options = permissionOptions(mode, false, bridge);
    assert.equal(options.permissionMode, "acceptEdits");
    assert.deepEqual(options.allowedTools, ["mcp__bridge__visualize", "mcp__bridge__notes_search", "mcp__bridge__notes_read", "mcp__bridge__memory_search", "mcp__bridge__settings_read", "mcp__bridge__memory_propose", "mcp__bridge__outgoing_draft", "mcp__bridge__settings_change"]);
    assert.ok(!options.allowedTools.some(name => /notes_(create|edit|append|set|draw)/.test(name)));
  }
  assert.equal(permissionOptions("Shared", false, {}).allowedTools, undefined, "no bridge server, nothing to allow");
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

test("a chat carries Bridge's stdio visualize server through to the SDK", () => {
  const mcpServers = { bridge: { type: "stdio", command: "/Applications/Bridge.app/Contents/MacOS/bridged", args: ["--bridge-mcp-visualize"] } };
  const options = buildOptions({ ...base, writeMode: null, instructions: null, resume: false, mcpServers });
  assert.deepEqual(options.mcpServers, mcpServers);
  assert.equal(options.strictMcpConfig, false);
  assert.equal(options.permissionMode, "bypassPermissions");
});
