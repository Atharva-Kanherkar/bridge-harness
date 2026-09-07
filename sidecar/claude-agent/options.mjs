import { briefingOptions, isBriefing } from "./briefing.mjs";

export function permissionOptions(mode) {
  switch (mode) {
    case "ReadOnly":
      return {
        permissionMode: "dontAsk",
        allowedTools: ["Read", "Grep", "Glob", "Bash"],
        disallowedTools: ["Edit", "Write", "NotebookEdit"],
      };
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

export function buildOptions({ sessionId, model, cwd, resume, instructions, writeMode, plugins = [], mcpServers = {}, briefing = null, effort = null }) {
  // A briefing run replaces the permission half of these options wholesale. It is
  // not a stricter write mode, so it does not layer on top of one — see
  // briefing.mjs and bridge-core/src/briefing_policy.rs.
  const authority = isBriefing({ briefing })
    ? briefingOptions(briefing, mcpServers)
    : {
        // Provider discovery supplies enabled plugin paths and credential-free
        // connector endpoints explicitly. Keep project/local settings, but do not
        // inherit unrelated global hooks, permissions, or inline credentials.
        settingSources: ["project", "local"],
        strictMcpConfig: false,
        mcpServers,
        plugins: plugins.map(path => ({ type: "local", path })),
        ...permissionOptions(writeMode),
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
