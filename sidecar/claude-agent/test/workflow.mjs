import test from "node:test";
import assert from "node:assert/strict";
import { makeWorkflowGate, briefingOptions, connectorsToAwait } from "../briefing.mjs";
import { createAskRelay } from "../ask.mjs";
import { buildOptions, WORKFLOW_ESCAPE_TOOLS } from "../options.mjs";
import { allConnected, connectorUnavailableFrame, settleWorkflowConnectors } from "../workflow-connectors.mjs";
import { connectorStatus, permissionDecision, query } from "../../../testing/fixtures/workflow-e2e/fake-sdk.mjs";
import { withChildPermissions, childPermissions } from "../subagents.mjs";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";

const servers = [
  { key: "claude.ai Slack", wire: "claude_ai_Slack", read: true, write: "ask" },
  { key: "claude.ai Linear", wire: "claude_ai_Linear", read: true, write: "never" },
];

test("the workflow gate allows a read, asks for a write, and denies everything else", async () => {
  const asked = [];
  const gate = makeWorkflowGate({
    servers,
    ask: async (tool) => { asked.push(tool); return tool.endsWith("send_message") ? "allow" : "deny"; },
  });
  assert.equal((await gate("mcp__claude_ai_Slack__slack_read_thread", {}, { toolUseID: "t1" })).behavior, "allow");
  assert.equal((await gate("mcp__claude_ai_Linear__list_issues", {})).behavior, "allow");
  assert.deepEqual(asked, [], "reads never ask");

  assert.equal((await gate("mcp__claude_ai_Slack__slack_send_message", { text: "hi" })).behavior, "allow");
  const declined = await gate("mcp__claude_ai_Slack__slack_delete_message", {});
  assert.equal(declined.behavior, "deny");
  assert.match(declined.message, /slack_delete_message/);
  assert.deepEqual(asked, ["mcp__claude_ai_Slack__slack_send_message", "mcp__claude_ai_Slack__slack_delete_message"]);

  // A write on a read-only server, an ungranted server, and a built-in are denied by name.
  for (const tool of ["mcp__claude_ai_Linear__create_issue", "mcp__claude_ai_Gmail__send_email", "Bash"]) {
    const denied = await gate(tool, {});
    assert.equal(denied.behavior, "deny", tool);
    assert.ok(denied.message.includes(tool), denied.message);
  }
  assert.equal(asked.length, 2, "denied calls never reach Bridge");
});

test("a granted connector with camelCase tool names asks for them instead of refusing the server", async () => {
  const asked = [];
  const atlassian = [
    { key: "claude.ai Atlassian", wire: "claude_ai_Atlassian", read: true, write: "ask" },
    { key: "claude.ai Asana", wire: "claude_ai_Asana", read: true, write: "never" },
  ];
  const gate = makeWorkflowGate({ servers: atlassian, ask: async (tool) => { asked.push(tool); return "allow"; } });
  assert.equal((await gate("mcp__claude_ai_Atlassian__createJiraIssue", { summary: "x" })).behavior, "allow");
  // Not a read: its words are not trusted, so it asks rather than running unasked.
  assert.equal((await gate("mcp__claude_ai_Atlassian__getJiraIssue", {})).behavior, "allow");
  assert.deepEqual(asked, ["mcp__claude_ai_Atlassian__createJiraIssue", "mcp__claude_ai_Atlassian__getJiraIssue"]);
  // On a read-only grant the same name is refused as a write, by name.
  const denied = await gate("mcp__claude_ai_Asana__getTask", {});
  assert.equal(denied.behavior, "deny");
  assert.match(denied.message, /is not granted to this workflow/);
  assert.equal(asked.length, 2);
});

test("a briefing never reads a camelCase name as a scoped read", async () => {
  const options = briefingOptions({ readScopeServers: ["claude.ai Atlassian"], allowedTools: [] }, {});
  assert.equal((await options.canUseTool("mcp__claude_ai_Atlassian__getJiraIssue", {})).behavior, "deny");
  assert.equal((await options.canUseTool("mcp__claude_ai_Atlassian__get_issue", {})).behavior, "allow");
});

test("a read verb beside a mutation word is a write and asks", async () => {
  const asked = [];
  const gate = makeWorkflowGate({ servers, ask: async (tool) => { asked.push(tool); return "deny"; } });
  assert.equal((await gate("mcp__claude_ai_Slack__get_or_create_channel", {})).behavior, "deny");
  assert.deepEqual(asked, ["mcp__claude_ai_Slack__get_or_create_channel"]);
});

test("workflow options never pre-approve, load account connectors natively, and have no built-ins", () => {
  const options = briefingOptions(
    { workflow: { servers }, deniedBuiltins: ["Bash", "Write"], maxArgumentBytes: 4096 },
    { "claude.ai Slack": { type: "http" }, "claude.ai Linear": { type: "http" }, "claude.ai Gmail": { type: "http" } },
    async () => "deny",
  );
  assert.equal(options.allowedTools, undefined);
  // An explicit copy of a claude.ai connector has no sign-in: it reads
  // needs-auth with no tools. Claude Code loads them itself instead.
  assert.equal(options.strictMcpConfig, false);
  assert.deepEqual(options.mcpServers, {});
  // No built-in tool at all: several run without asking canUseTool.
  assert.deepEqual(options.tools, []);
  assert.deepEqual(options.disallowedTools, ["Bash", "Write"]);
  assert.deepEqual(options.settingSources, []);
  assert.deepEqual(options.plugins, []);
  assert.equal(typeof options.canUseTool, "function");
});

test("a workflow granting only servers it is handed keeps them explicit and strict", () => {
  const local = [{ key: "qa-notes", wire: "qa-notes", read: true, write: "ask" }];
  const options = briefingOptions(
    { workflow: { servers: local } },
    { "qa-notes": { type: "stdio", command: "node" }, "claude.ai Gmail": { type: "http" } },
    async () => "deny",
  );
  assert.equal(options.strictMcpConfig, true);
  assert.deepEqual(Object.keys(options.mcpServers), ["qa-notes"]);
  assert.deepEqual(options.tools, []);
});

test("buildOptions denies every configured server a connector workflow was not granted", () => {
  const configured = { "claude.ai Slack": { type: "http" }, "claude.ai Linear": { type: "http" }, "claude.ai Gmail": { type: "http" }, "claude.ai Notion": { type: "http" } };
  const options = buildOptions({ briefing: { workflow: { servers }, deniedBuiltins: [] }, mcpServers: configured, effort: "ultracode" });
  assert.deepEqual(options.settings.deniedMcpServers.map((entry) => entry.serverName).sort(), ["claude.ai Gmail", "claude.ai Notion"]);
  assert.equal(options.settings.ultracode, true, "flag settings keep what they already carried");
  assert.deepEqual(options.tools, []);
  assert.equal(options.strictMcpConfig, false);

  // An ordinary chat is unchanged: no tool list, no denied servers.
  const chat = buildOptions({ writeMode: "Isolated", mcpServers: configured });
  assert.equal(chat.tools, undefined);
  assert.equal(chat.settings, undefined);
  // A worktree workflow reaches no server, so it names none.
  const worktree = buildOptions({ writeMode: "Isolated", briefing: { workflow: { servers: [], worktree: true } }, mcpServers: configured });
  assert.equal(worktree.settings, undefined);
});

test("the connector wait holds until granted connectors settle and switches off the rest", async () => {
  const calls = [];
  let polls = 0;
  const script = [
    new Error("not ready"),
    [],
    [{ name: "claude.ai Slack", status: "pending" }, { name: "claude.ai Gmail", status: "connected" }],
    [{ name: "claude.ai Slack", status: "connected" }, { name: "claude.ai Gmail", status: "connected" }, { name: "claude.ai Linear", status: "needs-auth" }],
  ];
  const run = {
    mcpServerStatus: async () => { const next = script[Math.min(polls++, script.length - 1)]; if (next instanceof Error) throw next; return next; },
    toggleMcpServer: async (name, enabled) => { calls.push([name, enabled]); },
  };
  let clock = 0;
  const result = await settleWorkflowConnectors(run, {
    granted: ["claude.ai Slack", "claude.ai Linear"],
    waitFor: ["claude.ai Slack", "claude.ai Linear"],
    now: () => clock,
    sleep: async (ms) => { clock += ms; },
  });
  assert.equal(result.ready, true);
  assert.equal(polls, 4, "a throwing call, an empty list and a pending server each keep it waiting");
  assert.deepEqual(calls, [["claude.ai Gmail", false]], "an ungranted server is switched off once");
  assert.deepEqual(result.switchedOff, ["claude.ai Gmail"]);
});

test("the connector wait gives up at its deadline and never rejects", async () => {
  let clock = 0;
  const run = { mcpServerStatus: async () => [{ name: "claude.ai Slack", status: "pending" }], toggleMcpServer: async () => { throw new Error("no"); } };
  const result = await settleWorkflowConnectors(run, { granted: ["claude.ai Slack"], waitFor: ["claude.ai Slack"], timeoutMs: 1000, pollMs: 250, now: () => clock, sleep: async (ms) => { clock += ms; } });
  assert.equal(result.ready, false);
  assert.equal(clock, 1000);
});

test("through the fake SDK, an explicit account copy reaches nothing and the native route connects", () => {
  const explicit = { strictMcpConfig: true, mcpServers: { "claude.ai Slack": { type: "http" } } };
  assert.deepEqual(connectorStatus(explicit), [{ name: "claude.ai Slack", status: "needs-auth" }]);
  const native = buildOptions({ briefing: { workflow: { servers }, deniedBuiltins: [] }, mcpServers: { "claude.ai Slack": {}, "claude.ai Linear": {}, "claude.ai Gmail": {} } });
  assert.deepEqual(connectorStatus(native).map((entry) => `${entry.name}=${entry.status}`).sort(), ["claude.ai Linear=connected", "claude.ai Slack=connected"]);
});

test("a worktree workflow keeps coding tools and loses every escape built-in", async () => {
  const options = buildOptions({ writeMode: "Isolated", briefing: { workflow: { servers: [], worktree: true } } });
  for (const tool of ["Bash", "Read", "Write", "Edit", "Skill"]) assert.ok(options.tools.includes(tool), tool);
  for (const tool of WORKFLOW_ESCAPE_TOOLS) assert.ok(!options.tools.includes(tool), tool);
  const hooks = options.hooks.PreToolUse.flatMap((entry) => entry.hooks);
  for (const tool_name of ["SendMessage", "RemoteTrigger", "PushNotification", "Workflow", "CronCreate", "EnterWorktree", "ListAgents"]) {
    const results = await Promise.all(hooks.map((hook) => hook({ tool_name })));
    assert.ok(results.some((result) => result.hookSpecificOutput?.permissionDecision === "deny"), tool_name);
  }
});

test("worktree workflows retain user Bash allows and deny connectors and delegation before settings allows", async () => {
  const cwd = mkdtempSync(join(tmpdir(), "workflow-worktree-"));
  try {
    mkdirSync(join(cwd, ".claude"));
    const tool = "mcp__claude_ai_Slack__slack_send_message";
    const settings = join(cwd, ".claude", "settings.json");
    writeFileSync(settings, JSON.stringify({ permissions: { allow: [tool, "mcp__*"] } }));
    const user = join(cwd, "user-config");
    mkdirSync(user);
    writeFileSync(join(user, "settings.json"), JSON.stringify({ permissions: { allow: ["mcp__*", "Task", "Agent", "Bash(node:*)"] } }));
    const options = buildOptions({
      cwd, projectTrusted: true, writeMode: "Isolated",
      plugins: ["plugin-with-connectors"],
      mcpServers: { "claude.ai Slack": { type: "http" }, bridge: { type: "stdio" } },
      briefing: { workflow: { servers: [], worktree: true } },
    });
    assert.equal(options.permissionMode, "acceptEdits");
    assert.equal(options.allowedTools, undefined);
    assert.deepEqual(options.settingSources, ["user"]);
    assert.equal(options.skills, "all");
    assert.deepEqual(options.plugins, []);
    assert.deepEqual(options.mcpServers, {});
    assert.equal(options.strictMcpConfig, true);
    assert.deepEqual(options.disallowedTools, ["mcp__*", "Task", "Agent"]);
    options.env = { CLAUDE_CONFIG_DIR: user };

    // Exercise the options after the sidecar adds its child-permission wrapper.
    // This fixture is a host that denies every prompt, so a call that reaches
    // the channel resolves instead of waiting for an answer nobody sends.
    const permissions = childPermissions(async (frame) => {
      permissions.respond({ response: { request_id: frame.request_id, response: { behavior: "deny" } } });
    }, "root");
    withChildPermissions(options, permissions);
    const hooks = options.hooks.PreToolUse.flatMap((entry) => entry.hooks);
    for (const agent_id of [undefined, "native-child"]) {
      for (const tool_name of [tool, "mcp__bridge__settings_change", "mcp__unknown__read", "Task", "Agent"]) {
        const results = await Promise.all(hooks.map((hook) => hook({ tool_name, agent_id })));
        assert.ok(results.some((result) => result.hookSpecificOutput?.permissionDecision === "deny"), `${agent_id}: ${tool_name}`);
      }
    }
    for (const tool_name of ["Read", "Write", "Edit", "Bash"]) {
      const results = await Promise.all(hooks.map((hook) => hook({ tool_name })));
      assert.ok(results.every((result) => result.hookSpecificOutput?.permissionDecision !== "deny"), tool_name);
    }

    // This negative control executes the connector without the worktree guards:
    // with the account's connectors loaded (no strict config), user settings
    // bypass the root canUseTool denial, as they do in the SDK.
    const unguarded = { ...options, hooks: {}, disallowedTools: [], strictMcpConfig: false };
    assert.equal((await permissionDecision(unguarded, tool)).behavior, "allow");
    const projectOnly = { ...unguarded, settingSources: ["project"] };
    assert.equal((await permissionDecision(projectOnly, tool)).behavior, "allow");
    assert.equal((await permissionDecision({ ...projectOnly, settingSources: [] }, tool)).behavior, "deny");
    assert.equal((await permissionDecision({ ...options, disallowedTools: [] }, tool)).behavior, "deny", "hook beats an inherited allow even without disallowedTools");

    async function* prompt() { yield { message: { content: "E2E:edit" } }; }
    for await (const _ of query({ prompt: prompt(), options })) {}
    assert.equal(readFileSync(join(cwd, "tests-ran.txt"), "utf8"), "passed", "Bash actually executes the test command");
    assert.equal(readFileSync(join(cwd, "CHANGELOG.md"), "utf8"), "- Workflows can edit code.\n");
    assert.throws(() => readFileSync(join(cwd, "writes.log")), /ENOENT/);
    for await (const _ of query({ prompt: prompt(), options: unguarded })) {}
    assert.match(readFileSync(join(cwd, "writes.log"), "utf8"), /ungranted connector write/, "the same fake writes when authority guards are removed");
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
});

test("the ask relay matches answers by id and denies what is pending when the session ends", async () => {
  const frames = [];
  const relay = createAskRelay(async (frame) => { frames.push(frame); });
  const first = relay.ask("mcp__claude_ai_Slack__slack_send_message", { text: "a" }, "tool-1");
  const second = relay.ask("mcp__claude_ai_Slack__slack_send_message", { text: "b" }, "tool-2");
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(frames.length, 2);
  assert.equal(frames[0].type, "control_request");
  assert.equal(frames[0].request.subtype, "can_use_tool");
  assert.deepEqual(frames[0].request.tool_input, { text: "a" });
  assert.equal(frames[0].request.tool_use_id, "tool-1");

  assert.equal(relay.resolve({ type: "control_response", response: { subtype: "success", request_id: frames[1].request_id, response: { behavior: "allow" } } }), true);
  assert.equal(await second, "allow");
  assert.equal(relay.resolve({ type: "control_response", response: { request_id: "wf-unknown", response: { behavior: "allow" } } }), false);

  relay.close();
  assert.equal(await first, "deny");
  assert.equal(await relay.ask("mcp__claude_ai_Slack__slack_send_message", {}), "deny", "nothing is asked after the session ends");
  assert.equal(frames.length, 2);
});

test("an interrupt denies what is waiting but later asks still go out", async () => {
  const frames = [];
  const relay = createAskRelay(async (frame) => { frames.push(frame); });
  const waiting = relay.ask("mcp__claude_ai_Slack__slack_send_message", {});
  relay.denyPending();
  assert.equal(await waiting, "deny");
  const later = relay.ask("mcp__claude_ai_Slack__slack_send_message", {});
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(relay.pendingCount, 1);
  relay.resolve({ type: "control_response", response: { request_id: frames.at(-1).request_id, response: { behavior: "allow" } } });
  assert.equal(await later, "allow");
});


test("workflow budgets reach the SDK and ordinary chats have no query ceilings", () => {
  const base = { writeMode: "Isolated", briefing: { workflow: { servers: [], worktree: true, budget: { maxTurns: 24, costCeilingMicrousd: 12_345 } } } };
  const options = buildOptions(base);
  assert.equal(options.maxTurns, 24);
  assert.equal(options.maxBudgetUsd, 0.012345);
  const noCost = buildOptions({ ...base, briefing: { workflow: { ...base.briefing.workflow, budget: { maxTurns: 1, costCeilingMicrousd: null } } } });
  assert.equal(noCost.maxTurns, 1);
  assert.equal(noCost.maxBudgetUsd, undefined);
  const ordinary = buildOptions({ writeMode: "Isolated" });
  assert.equal(ordinary.maxBudgetUsd, undefined);
  assert.equal(ordinary.maxTurns, undefined);
});

test("which launches wait for account connectors, and for which", () => {
  assert.deepEqual(connectorsToAwait({ workflow: { servers } }), { granted: ["claude.ai Slack", "claude.ai Linear"], waitFor: ["claude.ai Slack", "claude.ai Linear"] });
  assert.deepEqual(connectorsToAwait({ workflow: { servers, worktree: true } }).waitFor, [], "a worktree job reaches no connector");
  const ingress = { nativeConnectors: true, allowedServers: ["claude.ai Slack"], readScopeServers: ["claude.ai Slack"] };
  assert.deepEqual(connectorsToAwait(ingress), { granted: ["claude.ai Slack"], waitFor: ["claude.ai Slack"] });
  assert.deepEqual(connectorsToAwait({ ...ingress, nativeConnectors: false }).waitFor, [], "an explicit copy is not waited for");
  assert.deepEqual(connectorsToAwait({ ...ingress, directCall: true, exactAction: { tool: "t", arguments: {} } }).waitFor, [], "a direct call sends no turn");
  assert.deepEqual(connectorsToAwait({ nativeConnectors: true, readScopeServers: ["Local Echo"] }).waitFor, [], "only account connectors load natively");
  assert.deepEqual(connectorsToAwait(null).waitFor, []);
});

test("an unconnected connector becomes an error frame naming it and its state", () => {
  const statuses = [{ name: "claude.ai Slack", status: "needs-auth" }, { name: "claude.ai Linear", status: "connected" }];
  assert.equal(allConnected(["claude.ai Linear"], statuses), true);
  assert.equal(allConnected(["claude.ai Slack", "claude.ai Linear"], statuses), false);
  assert.equal(allConnected(["claude.ai Gmail"], statuses), false, "a connector that never loaded is not connected");
  const frame = connectorUnavailableFrame(["claude.ai Slack", "claude.ai Linear", "claude.ai Gmail"], statuses, "s1");
  assert.equal(frame.type, "result");
  assert.equal(frame.subtype, "error_connector_unavailable");
  assert.equal(frame.is_error, true);
  assert.equal(frame.session_id, "s1");
  assert.match(frame.result, /claude\.ai Slack \(needs-auth\)/);
  assert.match(frame.result, /claude\.ai Gmail \(not loaded\)/);
  assert.doesNotMatch(frame.result, /Linear/, "a connected one is not blamed");
});

test("an ephemeral briefing writes no transcript; nothing else changes", () => {
  assert.equal(buildOptions({ briefing: { allowedTools: [], ephemeral: true } }).persistSession, false);
  assert.equal(buildOptions({ briefing: { allowedTools: [] } }).persistSession, undefined);
  assert.equal(buildOptions({ writeMode: "Isolated", ephemeral: true }).persistSession, undefined, "a chat is never ephemeral");
});

/** Spawn the real sidecar over the fake SDK; return its frames once a result arrives. */
async function runSidecar(config, env = {}) {
  const here = fileURLToPath(new URL(".", import.meta.url));
  const child = spawn(process.execPath, [join(here, "../index.mjs"), JSON.stringify(config)], {
    stdio: ["pipe", "pipe", "inherit"],
    env: { ...process.env, BRIDGE_CLAUDE_SDK_ENTRY: join(here, "../../../testing/fixtures/workflow-e2e/fake-sdk.mjs"), ...env },
  });
  const frames = [];
  const done = new Promise((resolve) => {
    createInterface({ input: child.stdout }).on("line", (line) => {
      try { frames.push(JSON.parse(line)); } catch { return; }
      if (frames.at(-1).type === "result") resolve();
    });
    child.on("exit", resolve);
  });
  child.stdin.write(JSON.stringify({ type: "user", message: { role: "user", content: [{ type: "text", text: "Find what is new." }] } }) + "\n");
  const timer = setTimeout(() => child.kill("SIGKILL"), 20_000);
  await done;
  clearTimeout(timer);
  child.kill("SIGKILL");
  return frames;
}

const ingressConfig = {
  sessionId: "ingress-1", model: "haiku", cwd: tmpdir(),
  mcpServers: {},
  briefing: { allowedTools: [], allowedServers: ["claude.ai Slack"], readScopeServers: ["claude.ai Slack"], nativeConnectors: true, ephemeral: true, deniedBuiltins: [], maxArgumentBytes: 8192 },
};

test("a native briefing whose connector is signed out ends with an error frame and sends no turn", async () => {
  const frames = await runSidecar(ingressConfig, { FAKE_SDK_SIGNED_OUT: "claude.ai Slack" });
  const result = frames.find((frame) => frame.type === "result");
  assert.equal(result?.subtype, "error_connector_unavailable", JSON.stringify(frames));
  assert.match(result.result, /claude\.ai Slack \(needs-auth\)/);
  assert.equal(frames.filter((frame) => frame.type === "assistant").length, 0, "no model turn ran");
});

test("a native briefing whose connector connects runs its turn", async () => {
  const frames = await runSidecar(ingressConfig);
  const result = frames.find((frame) => frame.type === "result");
  assert.equal(result?.subtype, "success", JSON.stringify(frames));
  assert.ok(frames.some((frame) => frame.type === "assistant"));
});

test("any briefing's turn and cost ceilings reach the SDK; a chat gets none", () => {
  const ingress = buildOptions({ briefing: { allowedTools: [], budget: { maxTurns: 6, costCeilingMicrousd: 20_000 } } });
  assert.equal(ingress.maxTurns, 6);
  assert.equal(ingress.maxBudgetUsd, 0.02);
  const workflow = buildOptions({ briefing: { workflow: { servers: [], budget: { maxTurns: 3 } }, budget: { maxTurns: 99 } } });
  assert.equal(workflow.maxTurns, 3, "a workflow's own budget wins");
  const chat = buildOptions({ writeMode: "Isolated", budget: { maxTurns: 1 } });
  assert.equal(chat.maxTurns, undefined);
});
