/** Stop only the task named by Bridge, using the existing parent query. */
export async function stopTaskFrame(run, frame) {
  if (typeof frame.task_id !== "string" || !frame.task_id.trim()) throw new Error("stop_task needs a task_id");
  await run.stopTask(frame.task_id);
}

/** Bridge child approvals use the query's existing control channel. Parent
 * permission behavior stays as before: prompts that had no host are denied. */
export function childPermissions(writeFrame, sessionId) {
  const pending = new Map();
  let nextId = 0;
  return {
    async canUseTool(toolName, input, options) {
      if (!options.agentID) return { behavior: "deny", message: "Permission requires approval in Bridge." };
      const requestId = `child-permission-${++nextId}`;
      return new Promise((resolve) => {
        const finish = result => {
          pending.delete(requestId);
          options.signal.removeEventListener("abort", abort);
          resolve(result);
        };
        const abort = () => finish({ behavior: "deny", message: "Subagent permission cancelled." });
        pending.set(requestId, response => finish(response?.behavior === "allow"
          ? { behavior: "allow", updatedInput: input }
          : { behavior: "deny", message: "Permission declined in Bridge." }));
        options.signal.addEventListener("abort", abort, { once: true });
        if (options.signal.aborted) { abort(); return; }
        Promise.resolve(writeFrame({ type: "control_request", session_id: sessionId,
          request_id: requestId, request: { subtype: "can_use_tool", agentID: options.agentID,
            tool_use_id: options.toolUseID, tool_name: toolName, tool_input: input } })).catch(abort);
      });
    },
    respond(frame) {
      const response = frame?.response;
      const resolve = pending.get(response?.request_id);
      if (resolve) resolve(response.response);
    },
  };
}
