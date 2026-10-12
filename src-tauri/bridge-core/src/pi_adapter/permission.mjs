// Loaded explicitly by Bridge with all Pi extension discovery disabled.
// This gates authorization at Pi's tool_call boundary. It is not an OS sandbox.
export const READY = "bridge-pi-permissions-v1";
export default function bridgePermissions(pi) {
  pi.on("session_start", (_event, ctx) => {
    if (ctx.mode === "rpc" && ctx.hasUI) ctx.ui.notify(READY, "info");
  });
  pi.on("tool_call", async (event, ctx) => {
    try {
      if (ctx.mode !== "rpc" || !ctx.hasUI) return { block: true, reason: "Bridge permission channel unavailable" };
      const title = JSON.stringify({ bridgePermission: 1, toolCallId: event.toolCallId, toolName: event.toolName });
      const allowed = await ctx.ui.confirm(title, JSON.stringify(event.input), { timeout: 120000, signal: ctx.signal });
      if (allowed === true) return undefined;
    } catch {
      // An extension exception must never become an implicit tool grant.
    }
    return { block: true, reason: "Bridge denied tool permission" };
  });
  // /reload could replace the permission gate. Block user commands that bypass
  // model tool_call authorization, including direct !bash and extension reload.
  pi.on("input", event => {
    if (/^\s*[!/]/u.test(event.text)) return { action: "handled" };
    return { action: "continue" };
  });
  pi.on("user_bash", () => ({ result: { output: "Bridge requires tool approval", exitCode: 1, cancelled: false, truncated: false } }));
}
