import assert from "node:assert/strict";
import test from "node:test";

import { isMcpRead, makeReadOnlyHook, readOnlyOptions } from "../read-only.mjs";

test("read-only options expose intended reads instead of misusing allowedTools", () => {
  const local = readOnlyOptions({ networkAllowed: false, mcpServers: { notion: {} } });
  assert.equal(local.allowedTools, undefined);
  assert.equal(local.permissionMode, "default");
  assert.equal(local.permissionPrompts, "none");
  assert.deepEqual(local.settingSources, ["user"]);
  assert.equal(local.strictMcpConfig, true);
  assert.deepEqual(local.mcpServers, {});
  assert.deepEqual(local.plugins, []);
  assert.ok(local.tools.includes("Skill"));
  assert.ok(!local.tools.includes("Bash"));
  assert.ok(!local.tools.includes("WebFetch"));
  assert.deepEqual(local.disallowedTools, ["Edit", "Write", "NotebookEdit", "Task"]);
  assert.equal(typeof local.hooks.PreToolUse[0].hooks[0], "function");

  const networked = readOnlyOptions({ networkAllowed: true, mcpServers: { notion: {} } });
  assert.ok(networked.tools.includes("WebFetch"));
  assert.ok(networked.tools.includes("WebSearch"));
  assert.ok(networked.tools.includes("Bash"));
  assert.deepEqual(
    networked.mcpServers,
    { notion: {} },
    "a networked read-only worker keeps the connected servers Bridge projected",
  );
});

test("the Bridge hook allows local reads and explicitly denies mutation", async () => {
  const hook = makeReadOnlyHook({ networkAllowed: false });
  for (const tool_name of ["Read", "Grep", "Glob", "Skill", "TodoWrite"]) {
    const output = await hook({ hook_event_name: "PreToolUse", tool_name, tool_input: {} });
    assert.equal(output.hookSpecificOutput.permissionDecision, "allow", tool_name);
  }
  for (const tool_name of ["Edit", "Write", "Task", "mcp__notion__update_page", "unknown"]) {
    const output = await hook({ hook_event_name: "PreToolUse", tool_name, tool_input: {} });
    assert.equal(output.hookSpecificOutput.permissionDecision, "deny", tool_name);
    assert.match(output.hookSpecificOutput.permissionDecisionReason, /Bridge read-only policy denied/);
  }
});

test("MCP matching is fail-closed and network authorization is enforced", async () => {
  for (const name of [
    "mcp__notion__search",
    "mcp__github__list_issues",
    "mcp__docs__get-page",
    "mcp__db__query_rows",
  ]) assert.equal(isMcpRead(name), true, name);
  for (const name of [
    "mcp__notion__search_and_update",
    "mcp__notion__update_search_index",
    "mcp__notion__Search",
    "mcp__notion__search ",
    "mcp__broken",
  ]) assert.equal(isMcpRead(name), false, name);

  const offline = makeReadOnlyHook({
    networkAllowed: false,
    mcpServers: { notion: {} },
  });
  assert.equal(
    (await offline({ tool_name: "mcp__notion__search" })).hookSpecificOutput.permissionDecision,
    "deny",
    "no network route to any MCP server, projected or not",
  );
  const online = makeReadOnlyHook({
    networkAllowed: true,
    mcpServers: { notion: {} },
  });
  assert.equal(
    (await online({ tool_name: "mcp__notion__search" })).hookSpecificOutput.permissionDecision,
    "allow",
    "a read verb on a server Bridge actually connected is allowed",
  );
  assert.equal(
    (await online({ tool_name: "mcp__notion__create_page" })).hookSpecificOutput.permissionDecision,
    "deny",
    "a mutating verb stays denied even on a connected server",
  );
  assert.equal(
    (await online({ tool_name: "mcp__github__list_issues" })).hookSpecificOutput.permissionDecision,
    "deny",
    "a read verb on a server Bridge did not connect is not authority",
  );
  assert.equal(
    (await makeReadOnlyHook({ networkAllowed: true })({ tool_name: "mcp__notion__search" }))
      .hookSpecificOutput.permissionDecision,
    "deny",
    "a read-looking name is not authority without a reviewed connected server",
  );
});

test("a user allow rule cannot widen the Bridge hook decision", async () => {
  const inheritedUserAllow = new Set(["Write", "mcp__notion__create_page"]);
  const hook = makeReadOnlyHook({ networkAllowed: true });
  for (const tool_name of inheritedUserAllow) {
    assert.equal(
      (await hook({ tool_name })).hookSpecificOutput.permissionDecision,
      "deny",
      `${tool_name} must remain denied even when user settings allow it`,
    );
  }
});
