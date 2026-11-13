// A stand-in for @anthropic-ai/claude-agent-sdk, for the workflow end-to-end
// tests in src-tauri/bridged/tests/workflows.rs. Same `query()` shape as the
// SDK; the replies are scripted from markers in the run's instructions, and
// every write that actually executes is appended to `writes.log` in the run's
// folder, so a test can count them.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import { execFileSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const drafts = JSON.parse(readFileSync(join(here, "../workflow-drafts/cases.json"), "utf8")).cases;

function textOf(message) {
  const content = message?.message?.content;
  if (typeof content === "string") return content;
  return (content ?? []).filter((block) => block?.type === "text").map((block) => block.text).join("\n");
}

const FILE_EDIT_TOOLS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);

function matchesRule(rule, name, input) {
  if (rule === name || rule === "*" || (rule.endsWith("*") && name.startsWith(rule.slice(0, -1)))) return true;
  const bash = rule.match(/^Bash\((.*):\*\)$/);
  return name === "Bash" && bash && (input.command === bash[1] || input.command?.startsWith(`${bash[1]} `));
}

// Exercise the same authority boundary as the SDK: hooks precede settings
// allows, and an allowed call never asks canUseTool. Never read real settings
// in e2e: its sidecar wrapper supplies a disposable CLAUDE_CONFIG_DIR.
export async function permissionDecision(options, name, input = {}, context = {}) {
  for (const matcher of options.hooks?.PreToolUse ?? []) {
    for (const hook of matcher.hooks ?? []) {
      const result = await hook({ tool_name: name, tool_input: input, agent_id: context.agentID });
      if (result.hookSpecificOutput?.permissionDecision === "deny") {
        return { behavior: "deny", message: result.hookSpecificOutput.permissionDecisionReason };
      }
    }
  }
  if (options.disallowedTools?.some((rule) => matchesRule(rule, name, input))) return { behavior: "deny" };
  const cwd = options.cwd ?? process.cwd();
  const config = options.env?.CLAUDE_CONFIG_DIR ?? process.env.CLAUDE_CONFIG_DIR ?? join(homedir(), ".claude");
  const settings = { user: join(config, "settings.json"), project: join(cwd, ".claude/settings.json"), local: join(cwd, ".claude/settings.local.json") };
  const allow = [...(options.allowedTools ?? [])];
  for (const source of options.settingSources ?? []) {
    try { allow.push(...(JSON.parse(readFileSync(settings[source], "utf8")).permissions?.allow ?? [])); } catch {}
  }
  if (allow.some((rule) => matchesRule(rule, name, input))) return { behavior: "allow" };
  if (options.permissionMode === "bypassPermissions" || (options.permissionMode === "acceptEdits" && FILE_EDIT_TOOLS.has(name))) return { behavior: "allow" };
  return options.canUseTool ? options.canUseTool(name, input, context) : { behavior: "deny" };
}

const resultBlock = (status, summary) => `\n\`\`\`workflow-result\n${JSON.stringify({ status, summary })}\n\`\`\``;

export function query({ prompt, options = {} }) {
  let releaseHeldTurn = null;
  const sessionId = options.resume ?? options.sessionId ?? randomUUID();
  const cwd = options.cwd ?? process.cwd();
  const writes = join(cwd, "writes.log");
  const usage = { input_tokens: 120, output_tokens: 40 };
  const assistant = (content) => ({
    type: "assistant",
    session_id: sessionId,
    parent_tool_use_id: null,
    message: { id: `msg_${randomUUID()}`, type: "message", role: "assistant", model: options.model ?? "fake", content, usage },
  });
  const finish = (text) => [
    assistant([{ type: "text", text }]),
    { type: "result", subtype: "success", is_error: false, session_id: sessionId, result: text, num_turns: 1, duration_ms: 5, total_cost_usd: 0, usage },
  ];

  // The SDK's order: the tool call is streamed, then the gate is asked, then
  // the tool runs (or not) and its result is streamed.
  async function* tool(name, input, effect) {
    const id = `toolu_${randomUUID()}`;
    yield assistant([{ type: "tool_use", id, name, input }]);
    // As in the SDK, `canUseTool` is asked only when a call would prompt.
    // Under `acceptEdits` a file edit does not, so it runs without asking.
    const verdict = await permissionDecision(options, name, input, { toolUseID: id, signal: new AbortController().signal });
    const allowed = verdict?.behavior === "allow";
    if (allowed) effect?.();
    yield {
      type: "user",
      session_id: sessionId,
      parent_tool_use_id: null,
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: allowed ? "ok" : (verdict?.message ?? "denied"), is_error: !allowed }] },
    };
    return allowed;
  }

  async function* turn(text) {
    const brief = `${JSON.stringify(options.systemPrompt ?? "")}\n${text}`;
    const drafted = text.match(/The person wrote:\n"""\n([\s\S]*?)\n"""/);
    if (drafted) {
      // A draft turn: the reply the real model gave this sentence, recorded.
      const known = drafts.find((draft) => draft.sentence === drafted[1].trim());
      yield* finish(known?.reply ?? "{}");
      return;
    }
    if (brief.includes("E2E:live-budget")) {
      // A real sidecar emits pre-result usage, then stays active until Bridge
      // interrupts it. No result frame can supply usage for these runs.
      const message = { id: "live-budget-message", model: options.model, usage: { input_tokens: 100, output_tokens: 0 } };
      yield { type: "stream_event", session_id: sessionId, event: { type: "message_start", message } };
      yield { type: "stream_event", session_id: sessionId, event: { type: "message_delta", usage: { output_tokens: 8 } } };
      await new Promise((resolve) => { releaseHeldTurn = resolve; });
      return;
    }
    // A marker, or what a drafted "post … on Slack" job asks for (manual QA).
    if (brief.includes("E2E:post") || (/\bpost\b/i.test(brief) && /slack/i.test(brief))) {
      yield* tool("mcp__claude_ai_Linear__list_issues", { updatedSince: "yesterday" });
      const posted = yield* tool(
        "mcp__claude_ai_Slack__slack_send_message",
        { channel: "#team", text: "Yesterday: 2 issues moved to In review." },
        () => appendFileSync(writes, "slack_send_message\n"),
      );
      if (posted && brief.includes("E2E:hold")) {
        // A second write the person never answers: the run is mid-flight.
        yield* tool("mcp__claude_ai_Slack__slack_add_reaction", { channel: "#team", name: "white_check_mark" }, () => appendFileSync(writes, "slack_add_reaction\n"));
      }
      const summary = posted ? "Posted yesterday's Linear summary to #team." : "Would post yesterday's Linear summary to #team.";
      yield* finish(summary + resultBlock("done", summary));
      return;
    }
    if (brief.includes("E2E:edit")) {
      // A code workflow attempts a connector call even though it has no grants.
      // The real sidecar's pre-tool hook must refuse it before any effect.
      yield* tool("mcp__claude_ai_Slack__slack_send_message", { text: "ungranted" }, () => appendFileSync(writes, "ungranted connector write\n"));
      const file = join(cwd, "CHANGELOG.md");
      const edited = yield* tool("Write", { file_path: file, content: "- Workflows can edit code.\n" }, () => writeFileSync(file, "- Workflows can edit code.\n"));
      yield* tool("Bash", { command: "node -e \"require('node:fs').writeFileSync('tests-ran.txt', 'passed')\"" }, () => {
        execFileSync(process.execPath, ["-e", "require('node:fs').writeFileSync('tests-ran.txt', 'passed')"], { cwd });
      });
      const summary = edited ? "Added CHANGELOG.md." : "Could not write CHANGELOG.md.";
      yield* finish(summary + resultBlock(edited ? "done" : "not_done", summary));
      return;
    }
    yield* finish(`Nothing to do.${resultBlock("done", "Nothing to do.")}`);
  }

  const stream = (async function* () {
    yield { type: "system", subtype: "init", session_id: sessionId, cwd, model: options.model ?? "fake", tools: [], mcp_servers: [], permissionMode: "default", apiKeySource: "none" };
    for await (const message of prompt) yield* turn(textOf(message));
  })();
  return Object.assign(stream, {
    interrupt: async () => { releaseHeldTurn?.(); releaseHeldTurn = null; },
    close: () => {},
    supportedModels: async () => [],
  });
}
