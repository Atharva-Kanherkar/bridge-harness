/** Stop only the task named by Bridge, using the existing parent query. */
export async function stopTaskFrame(run, frame) {
  if (typeof frame.task_id !== "string" || !frame.task_id.trim()) throw new Error("stop_task needs a task_id");
  await run.stopTask(frame.task_id);
}

/** A run with its own gate (a briefing) already decides every call, a
 * subagent's included. The child prompt fills in only where nothing decides. */
export function withChildPermissions(options, permissions) {
  if (!options.canUseTool) options.canUseTool = permissions.canUseTool;
  return options;
}

/** Bridge's standing grant, scoped to this session and this call's own
 * suggestions. The SDK's suggestions may target local settings; forwarding
 * them verbatim would outlive the card's "for session" promise, so every
 * update is rewritten to `destination: "session"` and only allow rules are
 * copied. A mode change or a directory outside this call's suggestion list is
 * never invented here. */
export function sessionPermissionUpdates(suggestions) {
  if (!Array.isArray(suggestions)) return [];
  const updates = [];
  let rules = 0;
  for (const suggestion of suggestions) {
    if (!suggestion || typeof suggestion !== "object") continue;
    if (suggestion.type === "addRules" && Array.isArray(suggestion.rules)) {
      const allowed = [];
      for (const rule of suggestion.rules) {
        if (!rule || typeof rule.toolName !== "string" || !rule.toolName) continue;
        if (rules >= 100) break;
        allowed.push({
          toolName: rule.toolName,
          ...(typeof rule.ruleContent === "string" ? { ruleContent: rule.ruleContent } : {}),
        });
        rules += 1;
      }
      if (allowed.length) updates.push({ type: "addRules", rules: allowed, behavior: "allow", destination: "session" });
    } else if (suggestion.type === "addDirectories" && Array.isArray(suggestion.directories)) {
      const directories = suggestion.directories.filter((directory) => typeof directory === "string" && directory);
      if (directories.length) updates.push({ type: "addDirectories", directories, destination: "session" });
    }
  }
  return updates.slice(0, 32);
}

/** Bridge child and root approvals both use the query's existing control
 * channel. A Claude worker is the root query of its own process, so a request
 * with no SDK `agentID` still belongs to the person, not to a local deny: the
 * host renders the card and answers it. `agentID` is echoed only when the SDK
 * supplied one, which is what keeps a native child's card on its child row. */
export function childPermissions(writeFrame, sessionId) {
  const pending = new Map();
  let nextId = 0;
  let closed = false;
  const denyAll = (message) => {
    for (const [requestId, resolve] of pending) {
      pending.delete(requestId);
      resolve({ behavior: "deny", message });
    }
  };
  return {
    async canUseTool(toolName, input, options) {
      if (closed) return { behavior: "deny", message: "Permission channel closed." };
      const suggestions = Array.isArray(options.suggestions) ? options.suggestions : [];
      // The host offers "Allow for session" only when there is something the
      // SDK can persist; otherwise the button would be a one-shot in disguise.
      const allowSession = suggestions.length > 0;
      const requestId = `${options.agentID ? "child-permission" : "root-permission"}-${++nextId}`;
      return new Promise((resolve) => {
        const finish = result => {
          pending.delete(requestId);
          options.signal?.removeEventListener("abort", abort);
          resolve(result);
        };
        const abort = () => finish({ behavior: "deny", message: "Subagent permission cancelled." });
        pending.set(requestId, answer => {
          if (answer?.behavior !== "allow") return finish({ behavior: "deny", message: "Permission declined in Bridge." });
          const updatedPermissions = answer.decision === "acceptForSession"
            ? sessionPermissionUpdates(suggestions)
            : [];
          return finish({
            behavior: "allow",
            updatedInput: input,
            ...(updatedPermissions.length ? { updatedPermissions } : {}),
          });
        });
        options.signal?.addEventListener("abort", abort, { once: true });
        if (options.signal?.aborted) { abort(); return; }
        Promise.resolve(writeFrame({ type: "control_request", session_id: sessionId,
          request_id: requestId, request: { subtype: "can_use_tool",
            ...(options.agentID ? { agentID: options.agentID } : {}),
            allowSession, tool_use_id: options.toolUseID, tool_name: toolName, tool_input: input } })).catch(abort);
      });
    },
    respond(frame) {
      const response = frame?.response;
      const resolve = pending.get(response?.request_id);
      if (resolve) resolve(response.response);
    },
    /** A closed stdin means no answer can arrive; every waiting SDK prompt is
     * denied rather than left to hang the turn forever. */
    close() { closed = true; denyAll("Permission channel closed."); },
  };
}
