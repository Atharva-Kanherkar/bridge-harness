// A minimal stdio MCP server with one tool, `echo`. Every call is appended to
// the log file named by argv[2], so a test can prove what reached the server.
import { appendFileSync } from "node:fs";
import { createInterface } from "node:readline";

const log = process.argv[2];
const send = (message) => process.stdout.write(JSON.stringify(message) + "\n");

createInterface({ input: process.stdin }).on("line", (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: message.params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: {} },
        serverInfo: { name: "echo", version: "1" },
      },
    });
  } else if (message.method === "tools/list") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        tools: [{
          name: "echo",
          description: "Echo the text back.",
          inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
        }],
      },
    });
  } else if (message.method === "tools/call") {
    appendFileSync(log, JSON.stringify(message.params.arguments) + "\n");
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: { content: [{ type: "text", text: JSON.stringify({ echoed: message.params.arguments }) }] },
    });
  } else if (message.id !== undefined) {
    send({ jsonrpc: "2.0", id: message.id, result: {} });
  }
});
