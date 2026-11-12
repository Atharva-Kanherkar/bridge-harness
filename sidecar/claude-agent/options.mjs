import { briefingOptions, isBriefing } from "./briefing.mjs";
import { readOnlyOptions } from "./read-only.mjs";

/** The `bridge` server's tools that only read or display. */
export const BRIDGE_READ_TOOLS = ["mcp__bridge__visualize", "mcp__bridge__notes_search", "mcp__bridge__notes_read", "mcp__bridge__memory_search", "mcp__bridge__settings_read"];
/** The `bridge` server's tools whose only effect waits for the person's review:
 *  a memory proposal changes nothing until they approve it in Bridge, and a
 *  drafted message is a card the person edits and sends themselves. */
export const BRIDGE_REVIEWED_TOOLS = ["mcp__bridge__memory_propose", "mcp__bridge__outgoing_draft"];
/** Bridge's own settings, changed at the person's request: the theme, how
 *  chats show thinking, what a new chat starts on. The catalog cannot widen
 *  what an agent may do (no access, permissions, keys or budgets), every
 *  change shows in the transcript and is undone the same way, so asking first
 *  would only turn "make the theme light" into a refusal. */
export const BRIDGE_PREFERENCE_TOOLS = ["mcp__bridge__settings_change"];

/** Bridge tools only the chat itself may call. A Claude-native subagent
 *  inherits the parent's MCP tools and the root chat's token, so the core
 *  cannot tell its calls apart; the sidecar can, by the hook input's agent_id. */
export const BRIDGE_TOP_LEVEL_TOOLS = ["mcp__bridge__settings_read", "mcp__bridge__settings_change"];

/** PreToolUse runs before the allowlist and before bypass mode, so this holds
 *  in every write mode. agent_id is present only on a subagent's calls. */
export function makeTopLevelOnlyHook() {
  const guarded = new Set(BRIDGE_TOP_LEVEL_TOOLS);
  return async function topLevelOnlyPreToolUse(input = {}) {
    if (!input.agent_id || !guarded.has(input.tool_name)) return { continue: true };
    return {
      continue: true,
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason: "Only the chat itself changes Bridge's settings; report back to it instead.",
      },
    };
  };
}

export function permissionOptions(mode, networkAllowed = false, mcpServers = {}) {
  switch (mode) {
    case "ReadOnly":
      return readOnlyOptions({ networkAllowed, mcpServers });
    case "Shared":
    case "Isolated":
      // Bridge renders no Claude permission prompt, so anything that needs one
      // is refused outright. Bridge's own read-only tools change nothing and
      // run, and so does a memory proposal, which Bridge itself puts in front
      // of the person, and a change to Bridge's own settings; notes writes
      // stay refused until Bridge asks for them.
      return mcpServers.bridge
        ? { permissionMode: "acceptEdits", allowedTools: [...BRIDGE_READ_TOOLS, ...BRIDGE_REVIEWED_TOOLS, ...BRIDGE_PREFERENCE_TOOLS] }
        : { permissionMode: "acceptEdits" };
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

export function buildOptions({ sessionId, model, cwd, resume, instructions, writeMode, networkAllowed = false, plugins = [], mcpServers = {}, briefing = null, effort = null, projectTrusted = false }) {
  // A briefing run replaces the permission half of these options wholesale. It is
  // not a stricter write mode, so it does not layer on top of one — see
  // briefing.mjs and bridge-core/src/briefing_policy.rs.
  const authority = isBriefing({ briefing })
    ? briefingOptions(briefing, mcpServers)
    : {
        // User settings are required for user skills, commands, agents, and native
        // connector sign-in. Bridge's PreToolUse hook remains authoritative for
        // read-only workers even when user permissions contain broader allows.
        // A folder's own settings can run commands and widen permissions, so
        // they load only for a folder Claude Code trusts (see trust.mjs).
        settingSources: projectTrusted ? ["user", "project", "local"] : ["user"],
        skills: "all",
        strictMcpConfig: false,
        mcpServers,
        // Keep plugin skills/commands/agents, but fail closed on plugin-bundled
        // MCP servers: their OAuth context is not transferable to this SDK query.
        plugins: plugins.map(plugin => typeof plugin === "string"
          ? { type: "local", path: plugin, skipMcpDiscovery: true }
          : { type: "local", ...plugin }),
        ...permissionOptions(writeMode, networkAllowed, mcpServers),
      };
  if (!isBriefing({ briefing }) && mcpServers.bridge) {
    const existing = authority.hooks ?? {};
    authority.hooks = { ...existing, PreToolUse: [...(existing.PreToolUse ?? []), { hooks: [makeTopLevelOnlyHook()] }] };
  }
  // Reasoning effort is handled natively by the SDK; low/medium are honoured
  // rather than dropped the way the old thinking-budget mapping dropped them.
  const resolvedEffort = sdkEffort(effort);
  // A toolless briefing answers from text alone: no tool definitions are sent
  // at all, and its instructions replace the coding preset instead of riding
  // on it. Rust only sets this for a policy with an empty scope, and the
  // briefing gate still stands behind it.
  const toolless = isBriefing({ briefing }) && briefing.toolless === true;
  return {
    ...(resolvedEffort ? { effort: resolvedEffort } : {}),
    ...(model ? { model } : {}),
    ...(cwd ? { cwd } : {}),
    ...(resume && sessionId ? { resume: sessionId } : sessionId ? { sessionId } : {}),
    includePartialMessages: true,
    forwardSubagentText: true,
    perTaskStopAffordance: true,
    ...(instructions
      ? { systemPrompt: toolless ? instructions : { type: "preset", preset: "claude_code", append: instructions } }
      : {}),
    ...authority,
    ...(toolless ? { tools: [] } : {}),
  };
}

// Discovery needs model metadata only, not a project's hooks, plugins or MCP startup.
export function catalogOptions() {
  return { settingSources: [], strictMcpConfig: true, mcpServers: {}, plugins: [], tools: [] };
}
