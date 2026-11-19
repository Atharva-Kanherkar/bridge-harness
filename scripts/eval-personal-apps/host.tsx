// The eval's view host: Bridge's own McpAppFrame, exactly as a draft card
// mounts it (the same nested sandbox and CSP), with every tool call sent to
// the real daemon's `apps/preview_personal_app_tool`. Reads run live; writes
// come back as Bridge's "would send" preview and are never sent.
import { createRoot } from "react-dom/client";
import type { CallToolResult } from "@modelcontextprotocol/client";
import { McpAppFrame } from "../../src/mcp-apps/McpAppFrame";
import "../../src/index.css";

type View = { name: string; html: string; resourceUri: string; input: Record<string, unknown> };

const query = new URLSearchParams(location.search);
const id = query.get("id") ?? "";
const theme = query.get("theme") === "dark" ? "dark" : "light";
document.documentElement.classList.toggle("dark", theme === "dark");

async function callTool(tool: string, args: Record<string, unknown>): Promise<CallToolResult> {
  const response = await fetch("/eval-api/call", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id, tool, arguments: args }) });
  const answer = await response.json() as { result?: CallToolResult; error?: string };
  if (answer.error) throw new Error(answer.error);
  return answer.result!;
}

void fetch(`/eval-api/view?id=${encodeURIComponent(id)}`).then(response => response.json() as Promise<View>).then(view => {
  document.title = view.name;
  createRoot(document.getElementById("root")!).render(
    <div data-eval-host={id} className="overflow-hidden rounded-2xl border border-border bg-card">
      <McpAppFrame title={view.name} input={view.input} theme={theme} initialHeight={420}
        personalView={{ html: view.html, resourceUri: view.resourceUri }} onToolCall={callTool} />
    </div>,
  );
});
