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

export function makeReadOnlyHook({ networkAllowed = false, allowedMcpTools = [] } = {}) {
  const local = new Set(LOCAL_READ_TOOLS);
  const network = new Set(NETWORK_CAPABLE_TOOLS);
  const reviewedMcp = new Set(allowedMcpTools);
  return async function readOnlyPreToolUse(input = {}) {
    const toolName = input.tool_name;
    if (local.has(toolName)) {
      return decision("allow", `Bridge read-only policy allows ${toolName}`);
    }
    if (networkAllowed && network.has(toolName)) {
      return decision("allow", `Bridge read-only network policy allows ${toolName}`);
    }
    if (networkAllowed && reviewedMcp.has(toolName)) {
      return decision("allow", `Bridge read-only policy allows reviewed MCP tool ${toolName}`);
    }
    const reason = network.has(toolName) || isMcpRead(toolName)
      ? `Bridge read-only policy denied ${toolName} because this task has no network authorization`
      : `Bridge read-only policy denied mutating or unknown tool ${toolName ?? "<missing>"}`;
    return decision("deny", reason);
  };
}

export function readOnlyOptions({ networkAllowed = false, allowedMcpTools = [] } = {}) {
  const tools = networkAllowed
    ? [...LOCAL_READ_TOOLS, ...NETWORK_CAPABLE_TOOLS]
    : [...LOCAL_READ_TOOLS];
  return {
    // The isolated user source contains projected capabilities plus a sanitized
    // settings document. Project/local settings can contain executable hooks,
    // so a read-only worker never loads those sources directly.
    settingSources: ["user"],
    strictMcpConfig: true,
    mcpServers: {},
    plugins: [],
    permissionMode: "default",
    permissionPrompts: "none",
    tools,
    disallowedTools: [...DIRECT_WRITE_TOOLS],
    hooks: {
      PreToolUse: [{ hooks: [makeReadOnlyHook({ networkAllowed, allowedMcpTools })] }],
    },
  };
}
