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

export function buildOptions({ sessionId, model, cwd, resume, instructions, writeMode }) {
  return {
    ...(model ? { model } : {}),
    ...(cwd ? { cwd } : {}),
    ...(resume && sessionId ? { resume: sessionId } : sessionId ? { sessionId } : {}),
    // Bridge is a local, single-user desktop client. Load the same trusted user
    // plugins and connectors as Claude Code while leaving credentials entirely
    // in Claude's provider-owned settings and credential stores.
    settingSources: ["user", "project", "local"],
    strictMcpConfig: false,
    mcpServers: {},
    includePartialMessages: true,
    ...(instructions
      ? { systemPrompt: { type: "preset", preset: "claude_code", append: instructions } }
      : {}),
    ...permissionOptions(writeMode),
  };
}
