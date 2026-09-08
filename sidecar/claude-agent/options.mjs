import { briefingOptions, isBriefing } from "./briefing.mjs";
import { readOnlyOptions } from "./read-only.mjs";

export function permissionOptions(mode, networkAllowed = false, allowedMcpTools = []) {
  switch (mode) {
    case "ReadOnly":
      return readOnlyOptions({ networkAllowed, allowedMcpTools });
    case "Shared":
    case "Isolated":
      return { permissionMode: "acceptEdits" };
    case "Full":
    default:
      return {
        permissionMode: "bypassPermissions",
        allowDangerouslySkipPermissions: true,
      };
  }
}

// Effort levels the Claude Agent SDK's `Options.effort` accepts. Bridge routes
// an effort per session; anything outside this set (e.g. Codex's `ultra`) is
// dropped so the SDK falls back to the model's own default rather than erroring.
const SDK_EFFORT_LEVELS = new Set(["low", "medium", "high", "xhigh", "max"]);

export function sdkEffort(effort) {
  const value = typeof effort === "string" ? effort.trim().toLowerCase() : "";
  return SDK_EFFORT_LEVELS.has(value) ? value : null;
}

export function buildOptions({ sessionId, model, cwd, resume, instructions, writeMode, networkAllowed = false, allowedMcpTools = [], plugins = [], mcpServers = {}, briefing = null, effort = null }) {
  // A briefing run replaces the permission half of these options wholesale. It is
  // not a stricter write mode, so it does not layer on top of one — see
  // briefing.mjs and bridge-core/src/briefing_policy.rs.
  const authority = isBriefing({ briefing })
    ? briefingOptions(briefing, mcpServers)
    : {
        // User settings are required for user skills, commands, agents, and native
        // connector sign-in. Bridge's PreToolUse hook remains authoritative for
        // read-only workers even when user permissions contain broader allows.
        settingSources: ["user", "project", "local"],
        skills: "all",
        strictMcpConfig: false,
        mcpServers,
        // Keep plugin skills/commands/agents, but fail closed on plugin-bundled
        // MCP servers: their OAuth context is not transferable to this SDK query.
        plugins: plugins.map(plugin => typeof plugin === "string"
          ? { type: "local", path: plugin, skipMcpDiscovery: true }
          : { type: "local", ...plugin }),
        ...permissionOptions(writeMode, networkAllowed, allowedMcpTools),
      };
  // Reasoning effort is handled natively by the SDK; low/medium are honoured
  // rather than dropped the way the old thinking-budget mapping dropped them.
  const resolvedEffort = sdkEffort(effort);
  return {
    ...(resolvedEffort ? { effort: resolvedEffort } : {}),
    ...(model ? { model } : {}),
    ...(cwd ? { cwd } : {}),
    ...(resume && sessionId ? { resume: sessionId } : sessionId ? { sessionId } : {}),
    includePartialMessages: true,
    ...(instructions
      ? { systemPrompt: { type: "preset", preset: "claude_code", append: instructions } }
      : {}),
    ...authority,
  };
}

// Discovery needs model metadata only, not a project's hooks, plugins or MCP startup.
export function catalogOptions() {
  return { settingSources: [], strictMcpConfig: true, mcpServers: {}, plugins: [], tools: [] };
}
