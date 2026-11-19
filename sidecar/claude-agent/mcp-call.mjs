// One approved MCP tool call with no model turn.
//
// Rust launches the sidecar exactly as for a connector action turn (same
// briefing, same `exactAction`), then writes one stdin frame instead of a user
// turn:
//
//   stdin  {"type":"mcp_call","id":"…","tool":"mcp__claude_ai_Slack__slack_send_message","arguments":{…}}
//   stdout {"type":"mcp_call_result","id":"…","ok":true,"result":{content,isError?,structuredContent?}}
//          {"type":"mcp_call_result","id":"…","ok":false,"error":"…"}
//
// The frame is honoured only when it deep-equals the launch's approved
// `exactAction`, and only once. The SDK call also sets `check_permissions`, so
// Claude Code runs the briefing gate (`canUseTool`) on it exactly as it would a
// model's own call: two independent checks, either of which refuses.

import { isDeepStrictEqual } from "node:util";

/**
 * Build the frame handler.
 *
 * `request` is the query's control-request sender (`run.request.bind(run)`), and
 * `writeFrame` writes one stdout frame. Neither is touched for a refused frame
 * except to report the refusal.
 */
export function createDirectCall({ exactAction, request, writeFrame }) {
  let used = false;
  return async function handle(frame) {
    const id = typeof frame?.id === "string" && frame.id ? frame.id : null;
    const refuse = (error) => writeFrame({ type: "mcp_call_result", id, ok: false, error });
    if (!id) return refuse("a direct call needs an id");
    if (!exactAction) return refuse("this launch carries no approved call");
    if (used) return refuse("the approved call was already used");
    if (frame.tool !== exactAction.tool || !isDeepStrictEqual(frame.arguments, exactAction.arguments)) {
      return refuse("this call does not match the approved destination and payload");
    }
    used = true;
    let response;
    try {
      response = await request({
        subtype: "mcp_call",
        tool: exactAction.tool,
        arguments: exactAction.arguments,
        // Decided as the model's own call would be: the briefing gate runs.
        check_permissions: true,
        // Bridge parses the result itself; keep the server's own blocks.
        result_not_for_model: true,
      });
    } catch (error) {
      return refuse(String(error?.message ?? error));
    }
    const result = response?.response;
    if (!result || typeof result !== "object") return refuse("the harness returned no tool result");
    return writeFrame({ type: "mcp_call_result", id, ok: true, result });
  };
}
