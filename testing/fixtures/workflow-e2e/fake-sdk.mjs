// A stand-in for @anthropic-ai/claude-agent-sdk, for the workflow end-to-end
// tests in src-tauri/bridged/tests/workflows.rs. Same `query()` shape as the
// SDK; the replies are scripted from markers in the run's instructions, and
// every write that actually executes is appended to `writes.log` in the run's
// folder, so a test can count them.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const drafts = JSON.parse(readFileSync(join(here, "../workflow-drafts/cases.json"), "utf8")).cases;

function textOf(message) {
  const content = message?.message?.content;
  if (typeof content === "string") return content;
  return (content ?? []).filter((block) => block?.type === "text").map((block) => block.text).join("\n");
}

const resultBlock = (status, summary) => `\n\`\`\`workflow-result\n${JSON.stringify({ status, summary })}\n\`\`\``;

export function query({ prompt, options = {} }) {
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
    const verdict = options.canUseTool
      ? await options.canUseTool(name, input, { toolUseID: id, signal: new AbortController().signal })
      : { behavior: "allow" };
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
    if (brief.includes("E2E:post")) {
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
      const file = join(cwd, "CHANGELOG.md");
      const edited = yield* tool("Write", { file_path: file, content: "- Workflows can edit code.\n" }, () => writeFileSync(file, "- Workflows can edit code.\n"));
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
    interrupt: async () => {},
    close: () => {},
    supportedModels: async () => [],
  });
}
