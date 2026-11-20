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

test("a read-only worker cannot start a Claude subagent", async () => {
  for (const networkAllowed of [false, true]) {
    const options = readOnlyOptions({ networkAllowed, mcpServers: { notion: {} } });
    assert.ok(!options.tools.includes("Agent"), `networkAllowed=${networkAllowed}`);
    assert.ok(!options.tools.includes("Task"), `networkAllowed=${networkAllowed}`);
    // The hook that Bridge hands to the SDK, and one made on its own.
    const hooks = [options.hooks.PreToolUse[0].hooks[0], makeReadOnlyHook({ networkAllowed, mcpServers: { notion: {} } })];
    for (const hook of hooks) {
      for (const tool_name of ["Agent", "Task"]) {
        const output = await hook({ hook_event_name: "PreToolUse", tool_name, tool_input: { prompt: "Read the repository" } });
        assert.equal(output.hookSpecificOutput.permissionDecision, "deny", `${tool_name} networkAllowed=${networkAllowed}`);
        assert.match(output.hookSpecificOutput.permissionDecisionReason, /Bridge read-only policy denied mutating or unknown tool/);
      }
    }
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

test("an offline worker can write only inside its assigned output directory", async () => {
  const { mkdtemp, writeFile, symlink, mkdir } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const root = await mkdtemp(join(tmpdir(), "bridge-read-only-output-"));
  const output = join(root, "output");
  await mkdir(output);
  await writeFile(join(root, "secret.txt"), "host secret");
  const outside = join(root, "escape");
  await mkdir(outside);
  await symlink(join(root, "secret.txt"), join(output, "linked.json"));

  const options = readOnlyOptions({ networkAllowed: false, outputDir: output });
  assert.ok(options.tools.includes("Write"), "the artifact contract needs a write tool");
  assert.ok(!options.tools.includes("Bash"), "offline still means no shell");
  assert.deepEqual(options.disallowedTools, ["Edit", "NotebookEdit", "Task"]);

  const hook = makeReadOnlyHook({ networkAllowed: false, outputDir: output });
  const write = (file_path) => hook({ tool_name: "Write", tool_input: { file_path } });
  assert.equal((await write(join(output, "offline-proof.json"))).hookSpecificOutput.permissionDecision, "allow");
  assert.equal((await write("offline-proof.json")).hookSpecificOutput.permissionDecision, "allow", "relative paths resolve inside the output root");
  assert.equal((await write(join(root, "secret.txt"))).hookSpecificOutput.permissionDecision, "deny");
  assert.equal((await write(join(root, "escape", "report.json"))).hookSpecificOutput.permissionDecision, "deny");
  assert.equal((await write(join(output, "..", "secret.txt"))).hookSpecificOutput.permissionDecision, "deny");
  assert.equal((await write(join(output, "linked.json"))).hookSpecificOutput.permissionDecision, "deny", "a symlink must not redirect the write");
  assert.equal((await hook({ tool_name: "Edit", tool_input: { file_path: join(output, "report.json") } })).hookSpecificOutput.permissionDecision, "deny");

  const withoutOutput = makeReadOnlyHook({ networkAllowed: false });
  assert.equal((await withoutOutput({ tool_name: "Write", tool_input: { file_path: join(output, "x") } })).hookSpecificOutput.permissionDecision, "deny");
});
