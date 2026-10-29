// Asking Bridge whether a tool call may run.
//
// A workflow run's write calls are decided in Rust, which knows what this gate
// cannot: what the person approved, what already ran, whether this is a dry
// run. The question travels the way Claude Code's own permission prompts did
// over stream-json, so Bridge's existing normalization reads it as
// `permission.requested` and its existing `respond` answers it:
//
//   stdout → {"type":"control_request","request_id":"wf-1",
//             "request":{"subtype":"can_use_tool","tool_name":…,"tool_input":…}}
//   stdin  ← {"type":"control_response",
//             "response":{"subtype":"success","request_id":"wf-1","response":{"behavior":"allow"}}}
//
// An unanswered ask must never hang a run forever: ending the session, or an
// interrupt, answers every pending ask with deny.

export function createAskRelay(writeFrame) {
  let next = 0;
  let closed = false;
  const pending = new Map();
  return {
    /** Ask about one call. Resolves to "allow" or "deny"; never rejects. */
    ask(toolName, input, toolUseID) {
      if (closed) return Promise.resolve("deny");
      next += 1;
      const requestId = `wf-${next}`;
      return new Promise((resolve) => {
        pending.set(requestId, resolve);
        const frame = {
          type: "control_request",
          request_id: requestId,
          request: {
            subtype: "can_use_tool",
            tool_name: toolName,
            tool_input: input ?? {},
            ...(toolUseID ? { tool_use_id: toolUseID } : {}),
          },
        };
        Promise.resolve()
          .then(() => writeFrame(frame))
          .catch(() => {
            // A request Bridge never saw cannot be approved.
            if (pending.delete(requestId)) resolve("deny");
          });
      });
    },
    /** Apply a `control_response`. True when it answered a pending ask. */
    resolve(frame) {
      const requestId = frame?.response?.request_id;
      const answer = pending.get(requestId);
      if (!answer) return false;
      pending.delete(requestId);
      answer(frame?.response?.response?.behavior === "allow" ? "allow" : "deny");
      return true;
    },
    /** Deny everything waiting; later asks still go out. For an interrupt. */
    denyPending() {
      for (const answer of pending.values()) answer("deny");
      pending.clear();
    },
    /** Deny everything waiting and every later ask. For the end of the session. */
    close() {
      closed = true;
      this.denyPending();
    },
    get pendingCount() {
      return pending.size;
    },
  };
}
