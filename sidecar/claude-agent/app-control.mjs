// Personal-app previews use the existing query's MCP clients, never a model turn.
import { isReadToolName, wireServerName } from "./briefing.mjs";

const bytes = (value) => Buffer.byteLength(JSON.stringify(value), "utf8");
const object = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

export function createAppControl({ enabled, status, request, writeFrame }) {
  let active = 0;
  return async (frame) => {
    const id = typeof frame?.id === "string" && frame.id.length <= 100 ? frame.id : null;
    const reply = (ok, value) => writeFrame({ type: "app_control_result", id, ok, ...(ok ? { result: value } : { error: value }) });
    if (!id || !enabled) return reply(false, "app previews are unavailable on this launch");
    if (active >= 4) return reply(false, "too many preview requests");
    active++;
    try {
      const input = frame.request;
      if (!object(input) || !["inventory", "read"].includes(input.kind) || typeof input.server !== "string" || !input.server || input.server.length > 100) {
        return reply(false, "invalid preview request");
      }
      const servers = await status();
      const matches = servers.filter(server => wireServerName(server.name) === wireServerName(input.server));
      // A collision in the wire segment could route a read to another account.
      if (matches.length !== 1 || matches[0].status !== "connected") return reply(false, "connect one unambiguous MCP server in this chat first");
      const server = matches[0];
      const tools = (server.tools ?? []).map(tool => ({
        name: tool.name,
        readOnly: tool.annotations?.readOnly ?? null,
        destructive: tool.annotations?.destructive ?? null,
      }));
      if (input.kind === "inventory") {
        const result = { name: server.name, status: server.status, tools };
        if (bytes(result) > 256 * 1024) return reply(false, "server inventory exceeds the preview limit");
        return reply(true, result);
      }
      const tool = tools.find(tool => tool.name === input.tool);
      if (!tool || !isReadToolName(input.tool) || tool.readOnly === false || tool.destructive === true) return reply(false, "this tool is not a safe preview read");
      if (!object(input.arguments) || bytes(input.arguments) > 64 * 1024) return reply(false, "preview arguments must be an object within 64 KiB");
      const response = await request({
        subtype: "mcp_call",
        tool: `mcp__${wireServerName(server.name)}__${input.tool}`,
        arguments: input.arguments,
        // Both core and this gate permit only reads. No SDK dialog can hang a view.
        check_permissions: false,
        result_not_for_model: true,
      });
      if (!object(response?.response) || bytes(response.response) > 256 * 1024) return reply(false, "the preview result is missing or exceeds 256 KiB");
      return reply(true, response.response);
    } catch {
      // SDK errors can contain server config or credentials. Never relay them.
      return reply(false, "the MCP preview request failed; check the connection in this chat");
    } finally {
      active--;
    }
  };
}
