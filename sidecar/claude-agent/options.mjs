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

export function buildOptions({ sessionId, model, cwd, resume, instructions, writeMode, plugins = [], mcpServers = {} }) {
  return {
    ...(model ? { model } : {}),
    ...(cwd ? { cwd } : {}),
    ...(resume && sessionId ? { resume: sessionId } : sessionId ? { sessionId } : {}),
    // Provider discovery supplies enabled plugin paths and credential-free
    // connector endpoints explicitly. Keep project/local settings, but do not
    // inherit unrelated global hooks, permissions, or inline credentials.
    settingSources: ["project", "local"],
    strictMcpConfig: false,
    mcpServers,
    plugins: plugins.map(path => ({ type: "local", path })),
    includePartialMessages: true,
    ...(instructions
      ? { systemPrompt: { type: "preset", preset: "claude_code", append: instructions } }
      : {}),
    ...permissionOptions(writeMode),
  };
}
