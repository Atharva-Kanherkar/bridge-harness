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
//   2. `disallowedTools` — exact built-in identities from Rust, so the common case
//      is stripped from the request before it reaches the gate at all. Only exact
//      names work here: an entry the SDK does not recognize as a tool removes
//      nothing, and the tool stays in context to be attempted.
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

// The verbs that make a connector tool a read under a scoped policy. Matched as
// a *word*, anywhere in the name — MCP servers namespace by server first
// (`slack_read_thread`), so a prefix rule recognises no real connector tool.
// Mirrors READ_TOOL_VERBS in bridge-core/src/briefing_policy.rs — the Rust side
// is the authority, and a test on each side pins the same vocabulary. Fail
// closed: an unrecognised verb is not a read.
const READ_TOOL_VERBS = new Set(["search", "read", "list", "get", "query", "fetch", "find"]);
const MUTATION_WORDS = new Set([
  "add", "approve", "archive", "ban", "create", "delete", "destroy", "edit", "invite", "kick",
  "merge", "patch", "post", "publish", "put", "reject", "remove", "rename", "schedule", "send",
  "set", "update", "upload", "write",
]);

/** Split `mcp__<server>__<tool>` into its parts, or null. */
function splitWireName(toolName) {
  if (!toolName.startsWith("mcp__")) return null;
  const rest = toolName.slice("mcp__".length);
  const split = rest.indexOf("__");
  if (split <= 0) return null;
  const server = rest.slice(0, split);
  const original = rest.slice(split + 2);
  if (!original || original !== original.toLowerCase()) return null;
  return { server, bare: original, words: original.split(/[_-]/) };
}

/** Is `mcp__<server>__<tool>` a read-verb tool on a scoped server? */
function isScopedRead(toolName, readScopeServers) {
  if (!readScopeServers.size) return false;
  const parts = splitWireName(toolName);
  if (!parts || !readScopeServers.has(parts.server)) return false;
  if (parts.words.some((word) => MUTATION_WORDS.has(word))) return false;
  return parts.words.some((word) => READ_TOOL_VERBS.has(word));
}

/**
 * Is this the one write an approved connector action may perform?
 *
 * Reached only when Rust compiled an action policy, which it only does for a run
 * carrying an `AuthorizedAction` — i.e. after a human approved the literal text
 * being sent. The gate still checks independently rather than trusting that:
 * right server, the intent's own word present, no forbidden word anywhere.
 */
function isApprovedAction(toolName, actionScope) {
  if (!actionScope) return false;
  const parts = splitWireName(toolName);
  if (!parts || parts.server !== actionScope.server) return false;
  if (parts.words.some((word) => actionScope.forbiddenWords.has(word))) return false;
  return parts.words.some((word) => actionScope.permittedWords.has(word));
}

/**
 * The per-call gate. Synchronous in effect — it never awaits anything, because a
 * briefing run has nobody to wait for and a promise that resolves on human input
 * is a hung background job.
 */
export function makeBriefingGate({
  allowedTools = [],
  readScopeServers = [],
  actionScope = null,
  maxArgumentBytes = 8192,
} = {}) {
  // A Set, so matching is exact by construction rather than by a comparison
  // somebody might later relax into a prefix test.
  const reviewed = new Set(allowedTools);
  const scoped = new Set(readScopeServers);
  const action = actionScope
    ? {
        server: actionScope.server,
        permittedWords: new Set(actionScope.permittedWords ?? []),
        forbiddenWords: new Set(actionScope.forbiddenWords ?? []),
      }
    : null;
  return async function canUseTool(toolName, input, options = {}) {
    const toolUseID = options?.toolUseID;
    if (
      !reviewed.has(toolName) &&
      !isScopedRead(toolName, scoped) &&
      !isApprovedAction(toolName, action)
    ) {
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
  // `allowedServers` already carries the read-scope servers on the Rust side,
  // but the union is restated here so the reachable-server set and the gate's
  // scope cannot drift apart if one field is ever sent without the other.
  const allowedServers = new Set([
    ...(briefing?.allowedServers ?? []),
    ...(briefing?.readScopeServers ?? []),
    ...(briefing?.actionScope ? [briefing.actionScope.server] : []),
  ]);
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
      readScopeServers: briefing?.readScopeServers ?? [],
      actionScope: briefing?.actionScope ?? null,
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
