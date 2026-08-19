// Briefing authority, enforced at the SDK boundary.
//
// Bridge's Rust side decides *what* a briefing run may touch (see
// bridge-core/src/briefing_policy.rs) and hands the decision down as config. This
// file's whole job is to make that decision the only way a tool call can happen.
//
// Three mechanisms, in order of how much they are relied on:
//
//   1. `canUseTool` — the real gate. Every tool call arrives here and is denied
//      unless it is exactly one of the reviewed identities. Nothing is
//      pre-approved, precisely so that no call can bypass this function.
//   2. `disallowedTools` — an explicit deny-list of built-in families, so the
//      common case is refused before it reaches the gate at all.
//   3. Withheld options — no `onUserDialog`, no `supportedDialogKinds`, no
//      inherited setting sources, no plugins. The SDK documents that a dialog
//      kind not declared is never emitted, so withholding the declaration is what
//      makes an elicitation unable to park a run that has no human attached.

/// A denial the SDK will surface as a tool error, with the reason it was refused.
function deny(message, toolUseID) {
  return { behavior: "deny", message, ...(toolUseID ? { toolUseID } : {}) };
}

function allow(toolUseID) {
  return { behavior: "allow", ...(toolUseID ? { toolUseID } : {}) };
}

/**
 * The per-call gate. Synchronous in effect — it never awaits anything, because a
 * briefing run has nobody to wait for and a promise that resolves on human input
 * is a hung background job.
 */
export function makeBriefingGate({ allowedTools = [], maxArgumentBytes = 8192 } = {}) {
  // A Set, so matching is exact by construction rather than by a comparison
  // somebody might later relax into a prefix test.
  const reviewed = new Set(allowedTools);
  return async function canUseTool(toolName, input, options = {}) {
    const toolUseID = options?.toolUseID;
    if (!reviewed.has(toolName)) {
      return deny(
        `\`${toolName}\` is not one of the reviewed connector reads for this briefing run`,
        toolUseID,
      );
    }
    // Measured on the serialized form, which is what actually travels.
    let bytes;
    try {
      bytes = Buffer.byteLength(JSON.stringify(input ?? {}), "utf8");
    } catch {
      // Unserializable input cannot be bounded, so it cannot be sent.
      return deny(`the arguments for \`${toolName}\` could not be measured`, toolUseID);
    }
    if (bytes > maxArgumentBytes) {
      return deny(
        `the arguments for \`${toolName}\` are ${bytes} bytes, over the ${maxArgumentBytes}-byte limit`,
        toolUseID,
      );
    }
    return allow(toolUseID);
  };
}

/**
 * The SDK options a briefing run uses, replacing the write-mode permission
 * options entirely rather than layering on top of them.
 *
 * `mcpServers` is filtered to the connector instances the reviewed tools belong
 * to: a server nobody reviewed a tool from has no reason to be reachable.
 */
export function briefingOptions(briefing, mcpServers = {}) {
  const allowedServers = new Set(briefing?.allowedServers ?? []);
  const scopedMcpServers = Object.fromEntries(
    Object.entries(mcpServers).filter(([name]) => allowedServers.has(name)),
  );
  return {
    // `default` rather than `dontAsk` or `bypassPermissions`: the gate below is
    // only consulted when the SDK actually asks, and asking is what we want.
    permissionMode: "default",
    // Deliberately no `allowedTools`. Anything pre-approved there would skip the
    // gate, and with it the argument-size check.
    disallowedTools: briefing?.deniedBuiltins ?? [],
    canUseTool: makeBriefingGate({
      allowedTools: briefing?.allowedTools ?? [],
      maxArgumentBytes: briefing?.maxArgumentBytes,
    }),
    // Only the reviewed connectors, and only as declared here.
    strictMcpConfig: true,
    mcpServers: scopedMcpServers,
    // Nothing inherited. A project's settings can carry permissions and hooks,
    // and a briefing run must not gain authority from where it happens to be run.
    settingSources: [],
    plugins: [],
    // No dialog kinds are declared and no dialog callback is wired, so the CLI
    // emits none and every flow behind one degrades to its no-dialog behaviour.
    // This is the mechanism that makes a permission prompt or an MCP elicitation
    // unable to park a run with no human attached.
  };
}

/** Is this config a briefing run? */
export function isBriefing(config) {
  return Boolean(config?.briefing);
}
