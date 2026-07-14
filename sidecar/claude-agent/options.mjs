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
    // Never inherit global user settings or auto-loaded MCP servers. Project
    // and local instructions remain available to sessions launched in a repo.
    settingSources: ["project", "local"],
    strictMcpConfig: true,
    mcpServers: {},
    includePartialMessages: true,
    ...(instructions
      ? { systemPrompt: { type: "preset", preset: "claude_code", append: instructions } }
      : {}),
    ...permissionOptions(writeMode),
  };
}
