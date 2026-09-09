// Bridge-owned read-only authority. User settings may add hooks and permission
// conveniences, but this hook runs before each visible tool and can only narrow
// the capability surface Bridge selected.

const LOCAL_READ_TOOLS = ["Read", "Grep", "Glob", "Skill", "TodoWrite"];
const NETWORK_CAPABLE_TOOLS = ["Bash", "WebFetch", "WebSearch"];
const DIRECT_WRITE_TOOLS = ["Edit", "Write", "NotebookEdit", "Task"];
const READ_TOOL_VERBS = ["search", "read", "list", "get", "query", "fetch", "find"];
const MUTATION_WORDS = new Set([
  "add", "approve", "create", "delete", "edit", "merge", "patch", "post", "publish",
  "put", "reject", "remove", "send", "set", "update", "write",
]);

export function isMcpRead(toolName) {
  if (typeof toolName !== "string" || !toolName.startsWith("mcp__")) return false;
  const split = toolName.indexOf("__", "mcp__".length);
  if (split <= "mcp__".length) return false;
  const original = toolName.slice(split + 2);
  const bare = original.toLowerCase();
  if (original !== bare || bare.split(/[_-]/).some((word) => MUTATION_WORDS.has(word))) return false;
  return READ_TOOL_VERBS.some(
    (verb) => bare === verb || (bare.startsWith(verb) && ["_", "-"].includes(bare[verb.length])),
  );
}

/** The `<server>` in `mcp__<server>__<tool>`, or null for a malformed name. */
function mcpServerName(toolName) {
  const split = toolName.indexOf("__", "mcp__".length);
  return split > "mcp__".length ? toolName.slice("mcp__".length, split) : null;
}

function decision(permissionDecision, permissionDecisionReason) {
  return {
    continue: true,
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision,
      permissionDecisionReason,
    },
  };
}

export function makeReadOnlyHook({ networkAllowed = false, mcpServers = {} } = {}) {
  const local = new Set(LOCAL_READ_TOOLS);
  const network = new Set(NETWORK_CAPABLE_TOOLS);
  // The servers Bridge actually connected and handed to this worker (see
  // claude_adapter.rs's `launch_mcp_servers`). Reading is scoped to exactly
  // these — a server nobody projected has no reason to be reachable, and a
  // mutating verb on a projected server is still denied below.
  const reviewedServers = new Set(Object.keys(mcpServers));
  return async function readOnlyPreToolUse(input = {}) {
    const toolName = input.tool_name;
    if (local.has(toolName)) {
      return decision("allow", `Bridge read-only policy allows ${toolName}`);
    }
    if (networkAllowed && network.has(toolName)) {
      return decision("allow", `Bridge read-only network policy allows ${toolName}`);
    }
    if (networkAllowed && isMcpRead(toolName) && reviewedServers.has(mcpServerName(toolName))) {
      return decision("allow", `Bridge read-only policy allows reviewed MCP server read ${toolName}`);
    }
    const reason = network.has(toolName) || isMcpRead(toolName)
      ? `Bridge read-only policy denied ${toolName} because this task has no network authorization`
      : `Bridge read-only policy denied mutating or unknown tool ${toolName ?? "<missing>"}`;
    return decision("deny", reason);
  };
}

export function readOnlyOptions({ networkAllowed = false, mcpServers = {} } = {}) {
  const tools = networkAllowed
    ? [...LOCAL_READ_TOOLS, ...NETWORK_CAPABLE_TOOLS]
    : [...LOCAL_READ_TOOLS];
  // Without network there is no route to any MCP server regardless of what
  // Bridge projected, so the map is dropped rather than merely left unused.
  const reviewedMcpServers = networkAllowed ? mcpServers : {};
  return {
    // The isolated user source contains projected capabilities plus a sanitized
    // settings document. Project/local settings can contain executable hooks,
    // so a read-only worker never loads those sources directly.
    settingSources: ["user"],
    strictMcpConfig: true,
    mcpServers: reviewedMcpServers,
    plugins: [],
    permissionMode: "default",
    permissionPrompts: "none",
    tools,
    disallowedTools: [...DIRECT_WRITE_TOOLS],
    hooks: {
      PreToolUse: [{ hooks: [makeReadOnlyHook({ networkAllowed, mcpServers: reviewedMcpServers })] }],
    },
  };
}
