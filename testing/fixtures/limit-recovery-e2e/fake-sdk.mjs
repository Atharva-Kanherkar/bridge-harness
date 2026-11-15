// A stand-in for @anthropic-ai/claude-agent-sdk, for the usage-limit
// recovery end-to-end tests in src-tauri/bridged/tests/limit_recovery.rs.
//
// A user message containing `E2E:limit` fails the way Claude fails when the
// account is out: an error result whose text is the recorded banner. Every
// other message succeeds. Each turn appends `<model>\t<prompt>` to
// `limit-e2e.log` in the session's working directory, so the test can see
// which model each turn ran on and what Bridge sent.
import { appendFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { join } from "node:path";

function textOf(message) {
  const content = message?.message?.content;
  if (typeof content === "string") return content;
  return (content ?? []).filter((block) => block?.type === "text").map((block) => block.text).join("\n");
}

export function query({ prompt, options = {} }) {
  const sessionId = options.resume ?? options.sessionId ?? randomUUID();
  const cwd = options.cwd ?? process.cwd();
  const model = options.model ?? "default";
  const usage = { input_tokens: 10, output_tokens: 5 };

  async function* turn(text) {
    appendFileSync(join(cwd, "limit-e2e.log"), `${model}\t${text.replace(/\s+/g, " ")}\n`);
    if (text.includes("E2E:limit")) {
      const banner = process.env.LIMIT_E2E_BANNER ?? "You've hit your session limit · resets 7:50pm (Asia/Tokyo)";
      yield { type: "result", subtype: "success", is_error: true, session_id: sessionId, result: banner, num_turns: 1, duration_ms: 5, total_cost_usd: 0, usage };
      return;
    }
    const reply = `ok on ${model}`;
    yield {
      type: "assistant",
      session_id: sessionId,
      parent_tool_use_id: null,
      message: { id: `msg_${randomUUID()}`, type: "message", role: "assistant", model, content: [{ type: "text", text: reply }], usage },
    };
    yield { type: "result", subtype: "success", is_error: false, session_id: sessionId, result: reply, num_turns: 1, duration_ms: 5, total_cost_usd: 0, usage };
  }

  const stream = (async function* () {
    yield { type: "system", subtype: "init", session_id: sessionId, cwd, model, tools: [], mcp_servers: [], permissionMode: "default", apiKeySource: "none" };
    for await (const message of prompt) yield* turn(textOf(message));
  })();
  return Object.assign(stream, {
    interrupt: async () => {},
    close: () => {},
    supportedModels: async () => [],
    setModel: async () => {},
    setPermissionMode: async () => {},
  });
}
