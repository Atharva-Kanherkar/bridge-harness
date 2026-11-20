import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { tmpdir } from "node:os";
import { Writable, PassThrough } from "node:stream";
import test from "node:test";
import { isDeepStrictEqual } from "node:util";
import { query } from "@anthropic-ai/claude-agent-sdk";

import { buildOptions, withDeniedServers } from "../options.mjs";
import { childPermissions, withChildPermissions } from "../subagents.mjs";
import { contextUsageFrame } from "../usage.mjs";

// The tool choice of a chat only removes. Each launch case below starts the
// real query() of the SDK with a process stub, then reads the real CLI
// arguments and the first control request on stdin, and compares them with
// the same launch without a choice.

// The SDK warns at each launch that the bare Bridge allow rules come before
// canUseTool. That is true of each chat today and is the same with a choice,
// so the warning only hides the test output.
process.removeAllListeners("warning");
process.on("warning", (warning) => {
  if (warning.code !== "CLAUDE_SDK_CAN_USE_TOOL_SHADOWED") console.error(warning);
});

const WRITE_MODES = ["ReadOnly", "Shared", "Isolated", "Full"];

const chat = {
  sessionId: "11111111-1111-4111-8111-111111111111",
  model: "sonnet",
  cwd: tmpdir(),
  resume: false,
  instructions: "Bridge context for this chat.",
  networkAllowed: true,
  plugins: [],
  mcpServers: { bridge: { type: "stdio", command: "/bin/bridged", args: ["--mcp-apps"] } },
  timing: true,
};

// The value that Rust sends for a chat that leaves out two servers and keeps
// two skills.
const SELECTION = {
  deniedMcpServers: ["claude.ai Gmail", "chrome-devtools"],
  skills: ["review", "chrome-devtools-mcp:a11y-debugging"],
};

/** A process for spawnClaudeCodeProcess: it hands each stdin line to
 * `onLine` and exits when stdin closes or on a kill. */
function processStub(onLine) {
  const events = new EventEmitter();
  const stdout = new PassThrough();
  let buffer = "";
  const proc = {
    stdout,
    killed: false,
    exitCode: null,
    kill() {
      proc.killed = true;
      exit();
      return true;
    },
    on(event, listener) { events.on(event, listener); return proc; },
    once(event, listener) { events.once(event, listener); return proc; },
    off(event, listener) { events.off(event, listener); return proc; },
  };
  function exit() {
    if (proc.exitCode !== null) return;
    proc.exitCode = 0;
    stdout.end();
    setImmediate(() => events.emit("exit", 0, null));
  }
  proc.stdin = new Writable({
    write(chunk, _encoding, done) {
      buffer += chunk.toString();
      for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
        onLine(buffer.slice(0, end));
        buffer = buffer.slice(end + 1);
      }
      done();
    },
    final(done) {
      exit();
      done();
    },
  });
  return proc;
}

/** The CLI flags of a spawn by name. A flag that repeats keeps each value. */
function flagsOf(args) {
  const flags = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (!arg.startsWith("--")) continue;
    const split = arg.indexOf("=");
    let name = arg.slice(2);
    let value = null;
    if (split > 0) {
      name = arg.slice(2, split);
      value = arg.slice(split + 1);
    } else if (index + 1 < args.length && !args[index + 1].startsWith("--")) {
      value = args[index + 1];
      index += 1;
    }
    (flags[name] ??= []).push(value);
  }
  return flags;
}

/** Launch the real SDK query with the options that index.mjs builds for
 * `config`. Resolves with the spawn arguments, their flags, the request of
 * the first stdin control frame and the options. */
async function launch(config) {
  const options = withChildPermissions(
    buildOptions({ projectTrusted: false, ...config }, async () => "deny"),
    childPermissions(async () => {}, config.sessionId),
  );
  let spawned;
  let timer;
  const first = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error("the SDK wrote no control request")), 5_000);
    options.spawnClaudeCodeProcess = (spawn) => {
      spawned = spawn;
      return processStub((line) => resolve(JSON.parse(line)));
    };
  });
  const run = query({ prompt: (async function* () {})(), options });
  const pump = (async () => { try { for await (const _ of run) { /* nothing arrives */ } } catch { /* the stub exits */ } })();
  try {
    const frame = await first;
    assert.equal(frame.type, "control_request");
    assert.equal(frame.request.subtype, "initialize");
    return { args: spawned.args, flags: flagsOf(spawned.args), init: frame.request, options };
  } finally {
    clearTimeout(timer);
    run.close();
    await pump;
  }
}

const allowed = ({ flags }) => flags.allowedTools?.[0]?.split(",") ?? [];
const settings = ({ flags }) => (flags.settings ? JSON.parse(flags.settings[0]) : null);
const isSkillRule = (rule) => rule === "Skill" || rule.startsWith("Skill(");
const NAMED_SKILL = /^Skill\([^(),]+\)$/;

/** Each hook of the options by event: its matcher and the names of its
 * functions. */
const hookShape = (hooks) => Object.fromEntries(Object.entries(hooks ?? {}).map(([event, matchers]) => [
  event,
  matchers.map((matcher) => [matcher.matcher ?? null, matcher.hooks.map((hook) => hook.name)]),
]));

test("no selection leaves skills all and the flag settings unchanged", async () => {
  for (const writeMode of WRITE_MODES) {
    for (const speed of [null, "fast"]) {
      const baseline = await launch({ ...chat, writeMode, speed });
      assert.equal(baseline.options.skills, "all");
      assert.ok(allowed(baseline).includes("Skill"), `${writeMode}: "all" gives the Skill rule`);
      assert.deepEqual(settings(baseline), speed ? { fastMode: true } : null);
      assert.equal("skills" in baseline.init, false);
      // A choice that leaves nothing out, and values of the wrong shape,
      // launch with the same arguments and the same control request.
      for (const toolSelection of [{ deniedMcpServers: [] }, {}, null, "all", ["github"], { deniedMcpServers: "github", skills: "all" }]) {
        const same = await launch({ ...chat, writeMode, speed, toolSelection });
        assert.deepEqual(same.args, baseline.args, `${writeMode} ${JSON.stringify(toolSelection)}`);
        assert.deepEqual(same.init, baseline.init);
      }
    }
  }
});

test("a skill list gives Skill(name) allow rules that are a subset of Skill", async () => {
  for (const writeMode of WRITE_MODES) {
    const baseline = await launch({ ...chat, writeMode });
    const chosen = await launch({ ...chat, writeMode, toolSelection: { deniedMcpServers: [], skills: SELECTION.skills } });
    const before = allowed(baseline);
    const after = allowed(chosen);
    assert.ok(before.includes("Skill"));
    assert.equal(after.includes("Skill"), false, `${writeMode}: the bare Skill rule allows every skill`);
    assert.deepEqual(after.filter(isSkillRule), ["Skill(review)", "Skill(chrome-devtools-mcp:a11y-debugging)"]);
    assert.deepEqual(after.filter((rule) => !isSkillRule(rule)), before.filter((rule) => !isSkillRule(rule)));
    // Each new rule names one skill, so it allows less than the Skill rule.
    for (const rule of after.filter((rule) => !before.includes(rule))) assert.match(rule, NAMED_SKILL);
  }
});

test("an empty skill list gives no Skill allow rule", async () => {
  for (const writeMode of WRITE_MODES) {
    const baseline = await launch({ ...chat, writeMode });
    const chosen = await launch({ ...chat, writeMode, toolSelection: { deniedMcpServers: [], skills: [] } });
    assert.equal(chosen.options.skills.length, 0);
    assert.equal(allowed(chosen).some(isSkillRule), false, writeMode);
    assert.deepEqual(allowed(chosen), allowed(baseline).filter((rule) => rule !== "Skill"));
  }
  // With no other allow rule, the flag is gone.
  const full = await launch({ ...chat, writeMode: "Full", toolSelection: { skills: [] } });
  assert.equal(full.flags.allowedTools, undefined);
});

test("the init request carries the skill list only for a list", async () => {
  const baseline = await launch({ ...chat, writeMode: "Shared" });
  assert.equal("skills" in baseline.init, false);
  const all = await launch({ ...chat, writeMode: "Shared", toolSelection: { deniedMcpServers: ["github"] } });
  assert.equal("skills" in all.init, false);
  const list = await launch({ ...chat, writeMode: "Shared", toolSelection: { deniedMcpServers: [], skills: SELECTION.skills } });
  assert.deepEqual(list.init.skills, SELECTION.skills);
  const empty = await launch({ ...chat, writeMode: "Shared", toolSelection: { deniedMcpServers: [], skills: [] } });
  assert.deepEqual(empty.init.skills, []);
  for (const chosen of [all, list, empty]) {
    const { skills: _skills, ...rest } = chosen.init;
    assert.deepEqual(rest, baseline.init);
  }
});

test("omitted servers merge into deniedMcpServers and keep each existing deny", async () => {
  for (const writeMode of WRITE_MODES) {
    const flagged = { ...chat, writeMode, speed: "fast", effort: "ultracode" };
    const baseline = await launch(flagged);
    const chosen = await launch({ ...flagged, toolSelection: { deniedMcpServers: [...SELECTION.deniedMcpServers, "claude.ai Gmail"] } });
    const { deniedMcpServers, ...rest } = settings(chosen);
    assert.deepEqual(rest, settings(baseline), `${writeMode}: each other flag setting stays`);
    assert.deepEqual(rest, { fastMode: true, ultracode: true });
    assert.deepEqual(deniedMcpServers, [{ serverName: "claude.ai Gmail" }, { serverName: "chrome-devtools" }]);
    // The user tier still loads, so a deny in the user settings still holds.
    assert.deepEqual(chosen.flags["setting-sources"], baseline.flags["setting-sources"]);
  }
  const plain = await launch({ ...chat, writeMode: "Shared", toolSelection: { deniedMcpServers: ["github"] } });
  assert.deepEqual(settings(plain), { deniedMcpServers: [{ serverName: "github" }] });

  // The merge is a union: each existing entry stays, in its place.
  const existing = { fastMode: true, deniedMcpServers: [{ serverName: "github" }, { serverName: "linear", scope: "user" }] };
  assert.deepEqual(withDeniedServers(existing, ["linear", "notion", "notion"]), {
    fastMode: true,
    deniedMcpServers: [{ serverName: "github" }, { serverName: "linear", scope: "user" }, { serverName: "notion" }],
  });
  assert.equal(withDeniedServers(existing, ["github"]), existing);
  assert.equal(withDeniedServers(existing, []), existing);
  assert.equal(withDeniedServers(null, []), null);
});

test("the bridge server is never denied", async () => {
  for (const writeMode of WRITE_MODES) {
    const baseline = await launch({ ...chat, writeMode });
    const chosen = await launch({ ...chat, writeMode, toolSelection: { deniedMcpServers: ["bridge", " Bridge ", "BRIDGE", "github"] } });
    assert.deepEqual(settings(chosen), { deniedMcpServers: [{ serverName: "github" }] });
    assert.deepEqual(chosen.flags["mcp-config"], baseline.flags["mcp-config"]);
    const bridgeRules = (launched) => allowed(launched).filter((rule) => rule.startsWith("mcp__bridge__"));
    assert.deepEqual(bridgeRules(chosen), bridgeRules(baseline));
    const onlyBridge = await launch({ ...chat, writeMode, toolSelection: { deniedMcpServers: ["bridge"] } });
    assert.deepEqual(onlyBridge.args, baseline.args, writeMode);
  }
  assert.equal(withDeniedServers(null, ["bridge", " bridge"]), null);
});

test("permission mode, hooks and the bridge allow list do not change in any write mode", async () => {
  for (const writeMode of WRITE_MODES) {
    for (const projectTrusted of [false, true]) {
      const label = `${writeMode} trusted=${projectTrusted}`;
      const baseline = await launch({ ...chat, writeMode, projectTrusted, speed: "fast" });
      const chosen = await launch({ ...chat, writeMode, projectTrusted, speed: "fast", toolSelection: SELECTION });
      // Only the allow list and the flag settings differ.
      const names = new Set([...Object.keys(baseline.flags), ...Object.keys(chosen.flags)]);
      const changed = [...names].filter((name) => !isDeepStrictEqual(baseline.flags[name], chosen.flags[name]));
      assert.deepEqual(changed.sort(), ["allowedTools", "settings"], label);
      assert.deepEqual(chosen.flags["permission-mode"], baseline.flags["permission-mode"]);
      assert.deepEqual(chosen.flags["allow-dangerously-skip-permissions"], baseline.flags["allow-dangerously-skip-permissions"]);
      assert.equal(chosen.options.permissionMode, baseline.options.permissionMode);
      assert.deepEqual(hookShape(chosen.options.hooks), hookShape(baseline.options.hooks), label);
      assert.deepEqual(chosen.init.hooks, baseline.init.hooks, label);
      const mcpRules = (launched) => allowed(launched).filter((rule) => rule.startsWith("mcp__"));
      assert.deepEqual(mcpRules(chosen), mcpRules(baseline), label);
      // The allow set is a subset of the baseline: a new rule names one skill
      // where the baseline allowed each skill.
      for (const rule of allowed(chosen)) {
        assert.ok(allowed(baseline).includes(rule) || (NAMED_SKILL.test(rule) && allowed(baseline).includes("Skill")), `${label}: ${rule}`);
      }
      // The deny set holds the baseline, and each other flag setting stays.
      const { deniedMcpServers, ...rest } = settings(chosen);
      assert.deepEqual(rest, settings(baseline));
      assert.deepEqual(deniedMcpServers.map((entry) => entry.serverName), SELECTION.deniedMcpServers);
      assert.deepEqual(chosen.flags.disallowedTools, baseline.flags.disallowedTools);
    }
  }
});

test("briefing and workflow options ignore the selection", async () => {
  const deniedBuiltins = ["Read", "Write", "Edit", "Bash", "WebFetch", "WebSearch", "Task", "Skill"];
  const notion = { notion: { type: "http", url: "https://mcp.example/notion" } };
  const slack = { "claude.ai Slack": { type: "http", url: "https://mcp.example/slack" } };
  const runs = [
    { writeMode: "ReadOnly", mcpServers: notion, briefing: { allowedTools: ["mcp__notion__search"], allowedServers: ["notion"], deniedBuiltins, maxArgumentBytes: 256 } },
    { writeMode: "ReadOnly", mcpServers: {}, briefing: { allowedTools: [], allowedServers: [], deniedBuiltins, maxArgumentBytes: 256, toolless: true } },
    {
      writeMode: "Shared",
      mcpServers: slack,
      briefing: { workflow: { servers: [{ key: "claude.ai Slack", wire: "claude_ai_Slack", read: true, write: "ask" }], worktree: false, budget: { maxTurns: 8, costCeilingMicrousd: null } }, deniedBuiltins: ["Bash"], maxArgumentBytes: 4096 },
    },
    { writeMode: "Isolated", mcpServers: {}, briefing: { workflow: { servers: [], worktree: true, budget: { maxTurns: 24, costCeilingMicrousd: 12_345 } } } },
  ];
  for (const run of runs) {
    const label = JSON.stringify(run.briefing).slice(0, 60);
    const baseline = await launch({ ...chat, ...run, speed: "fast" });
    for (const toolSelection of [SELECTION, { deniedMcpServers: ["notion", "claude.ai Slack"], skills: [] }]) {
      const chosen = await launch({ ...chat, ...run, speed: "fast", toolSelection });
      assert.deepEqual(chosen.args, baseline.args, label);
      assert.deepEqual(chosen.init, baseline.init, label);
      assert.deepEqual(chosen.options.skills, baseline.options.skills, label);
      assert.deepEqual(chosen.options.settings, baseline.options.settings, label);
    }
  }
});

test("the context usage frame keeps the token figure of each skill", () => {
  const frontmatter = [
    ...Array.from({ length: 70 }, (_, index) => ({ name: `skill-${index}`, source: "userSettings", path: `/skills/${index}/SKILL.md`, tokens: index + 1 })),
    { name: "vercel-plugin:deploy", source: "plugin", pluginName: "vercel-plugin", pluginId: "vercel-plugin@vercel", path: "/p/deploy.md", tokens: 500 },
    { name: "vercel-plugin:deploy", source: "plugin", tokens: 450 },
    // Names that cannot be an identity: a control character, too many bytes,
    // none at all. The frame drops each one and does not cut it.
    { name: "bad\u0007name", tokens: 900 },
    { name: "x".repeat(129), tokens: 800 },
    { name: "", tokens: 700 },
    { tokens: 600 },
  ];
  const frame = contextUsageFrame({ totalTokens: 9_000, maxTokens: 200_000, skills: { totalSkills: 76, includedSkills: 71, tokens: 2_985, skillFrontmatter: frontmatter } });
  const { items, ...totals } = frame.skills;
  assert.deepEqual(totals, { included: 71, total: 76, tokens: 2_985 });
  assert.equal(items.length, 64);
  assert.deepEqual(items[0], { name: "vercel-plugin:deploy", tokens: 500 });
  assert.deepEqual(items.slice(1, 3), [{ name: "skill-69", tokens: 70 }, { name: "skill-68", tokens: 69 }]);
  assert.deepEqual(items.at(-1), { name: "skill-7", tokens: 8 });
  // One row for each name, with its largest figure. Each row holds only the name
  // and the figure: no path, source or plugin id crosses to Rust.
  assert.equal(items.filter((row) => row.name === "vercel-plugin:deploy").length, 1);
  assert.ok(items.every((row) => isDeepStrictEqual(Object.keys(row), ["name", "tokens"])));
  assert.ok(items.every((row) => row.name && !/[\x00-\x1f\x7f]/.test(row.name) && Buffer.byteLength(row.name) <= 128));

  // A filtered list: the one kept skill keeps its figure under its init name.
  const kept = contextUsageFrame({ totalTokens: 10, maxTokens: 100, skills: { totalSkills: 218, includedSkills: 1, tokens: 32, skillFrontmatter: [{ name: "asd-ste100:ste-check", source: "plugin", pluginName: "asd-ste100", tokens: 32 }] } });
  assert.deepEqual(kept.skills, { included: 1, total: 218, tokens: 32, items: [{ name: "asd-ste100:ste-check", tokens: 32 }] });
  const edge = contextUsageFrame({ totalTokens: 10, maxTokens: 100, skills: { totalSkills: 1, includedSkills: 1, tokens: 4, skillFrontmatter: [{ name: "y".repeat(128), tokens: 4 }] } });
  assert.equal(edge.skills.items[0].name.length, 128);
  const bare = contextUsageFrame({ totalTokens: 10, maxTokens: 100, skills: { totalSkills: 0, includedSkills: 0, tokens: 0 } });
  assert.deepEqual(bare.skills.items, []);
});
