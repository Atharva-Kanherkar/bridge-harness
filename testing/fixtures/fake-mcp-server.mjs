#!/usr/bin/env node
// A tiny stdio MCP server for personal-app tests: one annotated read, one
// annotated destructive write. Writes record what they were sent to
// $FAKE_MCP_LOG (when set) so a test can prove what reached the server.
import { appendFileSync } from "node:fs";
import { createInterface } from "node:readline";

const tools = [
  { name: "search_people", description: "Find a teammate", inputSchema: { type: "object", properties: { query: { type: "string" } } }, annotations: { readOnlyHint: true } },
  { name: "send_note", description: "Send a note", inputSchema: { type: "object", properties: { to: { type: "string" }, text: { type: "string" } } }, annotations: { destructiveHint: true } },
];
const write = (message) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
createInterface({ input: process.stdin }).on("line", (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  const { id, method, params } = message;
  if (method === "initialize") {
    write({ id, result: { protocolVersion: params?.protocolVersion ?? "2025-06-18", capabilities: { tools: {} }, serverInfo: { name: "fake", version: "1" } } });
  } else if (method === "tools/list") {
    write({ id, result: { tools } });
  } else if (method === "tools/call") {
    if (process.env.FAKE_MCP_LOG) appendFileSync(process.env.FAKE_MCP_LOG, `${JSON.stringify({ name: params?.name, arguments: params?.arguments ?? {} })}\n`);
    const text = params?.name === "search_people" ? `Found ${params?.arguments?.query ?? ""} Lovelace` : `Sent ${JSON.stringify(params?.arguments ?? {})}`;
    write({ id, result: { content: [{ type: "text", text }] } });
  } else if (id !== undefined) {
    write({ id, error: { code: -32601, message: "method not found" } });
  }
});
