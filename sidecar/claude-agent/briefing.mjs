import { isDeepStrictEqual } from "node:util";
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

export function isReadToolName(name) {
  if (typeof name !== "string" || !/^[a-z0-9_-]{1,64}$/.test(name)) return false;
  const words = name.split(/[_-]/);
  return !words.some(word => MUTATION_WORDS.has(word)) && words.some(word => READ_TOOL_VERBS.has(word));
}

/**
 * The segment the harness puts in tool names for a configured server: anything
 * outside `[A-Za-z0-9_-]` becomes `_`, so `claude.ai Slack` is `claude_ai_Slack`.
 * Mirrors `workflows::wire_server_name` in Rust.
 */
export function wireServerName(server) {
  return String(server).replace(/[^A-Za-z0-9_-]/g, "_");
}

/**
 * Split `mcp__<server>__<tool>` into its parts, or null. A tool name that is
 * not all lowercase (`getJiraIssue`) still names its server; `lowercase` says
 * whether its words can be read as a verb at all, and nothing that reads
 * words trusts a name that is not.
 */
function splitWireName(toolName) {
  if (!toolName.startsWith("mcp__")) return null;
  const rest = toolName.slice("mcp__".length);
  const split = rest.indexOf("__");
  if (split <= 0) return null;
  const server = rest.slice(0, split);
  const original = rest.slice(split + 2);
  if (!original) return null;
  return { server, bare: original, words: original.split(/[_-]/), lowercase: original === original.toLowerCase() };
}

/** Is `mcp__<server>__<tool>` a read-verb tool on a scoped server? */
function isScopedRead(toolName, readScopeServers) {
  if (!readScopeServers.size) return false;
  const parts = splitWireName(toolName);
  if (!parts?.lowercase || !readScopeServers.has(parts.server)) return false;
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
  if (!parts?.lowercase || parts.server !== actionScope.server) return false;
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
  exactAction = null,
  maxArgumentBytes = 8192,
} = {}) {
  // A Set, so matching is exact by construction rather than by a comparison
  // somebody might later relax into a prefix test.
  const reviewed = new Set(allowedTools);
  // Scopes arrive as configuration keys (`claude.ai Slack`) and tool names
  // carry the wire segment (`claude_ai_Slack`), so both sides compare as wire.
  const scoped = new Set(readScopeServers.map(wireServerName));
  const action = actionScope
    ? {
        server: wireServerName(actionScope.server),
        permittedWords: new Set(actionScope.permittedWords ?? []),
        forbiddenWords: new Set(actionScope.forbiddenWords ?? []),
      }
    : null;
  let consumed = false;
  return async function canUseTool(toolName, input, options = {}) {
    const toolUseID = options?.toolUseID;
    if (exactAction) {
      if (consumed || toolName !== exactAction.tool || !isDeepStrictEqual(input, exactAction.arguments)) {
        return deny("this call does not match the one-use approved destination and payload", toolUseID);
      }
      if (Buffer.byteLength(JSON.stringify(input), "utf8") > maxArgumentBytes) return deny("approved arguments exceed the limit", toolUseID);
      consumed = true;
      return allow(toolUseID);
    }
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
 * The per-call gate for a workflow run with no built-in tools.
 *
 * Unlike a briefing, a workflow may write, so it is not synchronous in effect:
 * a write waits on `ask`, which Bridge answers. Reads on a read-granted server
 * run; any other call on a server granted `write: "ask"` asks; everything else
 * is denied by name. Servers are matched by their wire form, the segment the
 * harness puts in `mcp__<server>__<tool>`.
 */
export function makeWorkflowGate({ servers = [], maxArgumentBytes = 8192, ask = null } = {}) {
  const byWire = new Map(servers.map((server) => [server.wire, server]));
  return async function canUseTool(toolName, input, options = {}) {
    const toolUseID = options?.toolUseID;
    const parts = splitWireName(toolName);
    const server = parts ? byWire.get(parts.server) : undefined;
    if (!server) {
      return deny(`\`${toolName}\` is not a connector this workflow may use`, toolUseID);
    }
    let bytes;
    try {
      bytes = Buffer.byteLength(JSON.stringify(input ?? {}), "utf8");
    } catch {
      return deny(`the arguments for \`${toolName}\` could not be measured`, toolUseID);
    }
    if (bytes > maxArgumentBytes) {
      return deny(`the arguments for \`${toolName}\` are ${bytes} bytes, over the ${maxArgumentBytes}-byte limit`, toolUseID);
    }
    // A name whose words cannot be read (`createJiraIssue`) is not a read:
    // it asks like any write, rather than being refused outright.
    const isRead = parts.lowercase
      && !parts.words.some((word) => MUTATION_WORDS.has(word))
      && parts.words.some((word) => READ_TOOL_VERBS.has(word));
    if (isRead && server.read) return allow(toolUseID);
    if (server.write === "ask" && typeof ask === "function") {
      const behavior = await ask(toolName, input, toolUseID);
      return behavior === "allow"
        ? allow(toolUseID)
        : deny(`\`${toolName}\` was not approved for this run`, toolUseID);
    }
    return deny(`\`${toolName}\` is not granted to this workflow`, toolUseID);
  };
}

/** A launch that makes one approved call with no model turn. */
export function isDirectCall(briefing) {
  return briefing?.directCall === true && Boolean(briefing?.exactAction);
}

/**
 * A launch whose claude.ai connectors Claude Code loads itself, signed in: a
 * direct call, or a run Rust marked `nativeConnectors` (the Outgoing resolver).
 */
export function loadsConnectorsNatively(briefing) {
  return briefing?.nativeConnectors === true || isDirectCall(briefing) || isAppRuntime(briefing);
}

/**
 * A personal-app connection runtime: no model turn and no tools, only app
 * control (inventory and reads) on `appRuntime.server`. See app-control.mjs.
 */
export function isAppRuntime(briefing) {
  return typeof briefing?.appRuntime?.server === "string" && briefing.appRuntime.server.length > 0;
}

/**
 * The SDK options a briefing run uses, replacing the write-mode permission
 * options entirely rather than layering on top of them.
 *
 * `mcpServers` is filtered to the connector instances the reviewed tools belong
 * to: a server nobody reviewed a tool from has no reason to be reachable.
 */
export function briefingOptions(briefing, mcpServers = {}, ask = null, projectTrusted = false) {
  if (briefing?.workflow) return workflowOptions(briefing, mcpServers, ask);
  // `allowedServers` already carries the read-scope servers on the Rust side,
  // but the union is restated here so the reachable-server set and the gate's
  // scope cannot drift apart if one field is ever sent without the other.
  const allowedServers = new Set([
    ...(briefing?.allowedServers ?? []),
    ...(briefing?.readScopeServers ?? []),
    ...(briefing?.actionScope ? [briefing.actionScope.server] : []),
    ...(isAppRuntime(briefing) ? [briefing.appRuntime.server] : []),
  ]);
  // Native connectors: Claude Code loads its claude.ai connectors itself, with
  // the account's own sign-in. An explicit copy never shares that sign-in and
  // comes up needing authentication, so a run handed one can reach nothing.
  // This changes the route only: the gate below decides every call exactly as
  // before, and a direct call is further held to its one approved call by
  // mcp-call.mjs.
  const native = loadsConnectorsNatively(briefing);
  const scopedMcpServers = Object.fromEntries(
    Object.entries(mcpServers).filter(([name]) => allowedServers.has(name) && !(native && name.startsWith("claude.ai "))),
  );
  return {
    // `default` rather than `dontAsk` or `bypassPermissions`: the gate below is
    // only consulted when the SDK actually asks, and asking is what we want.
    permissionMode: "default",
    // No built-in tool at all. A briefing reads connectors and nothing else,
    // and several built-ins (SendMessage, RemoteTrigger, CronCreate) run
    // without asking `canUseTool`, so a deny-list cannot keep them out of a
    // run that reads text other people wrote.
    tools: [],
    // Deliberately no `allowedTools`. Anything pre-approved there would skip the
    // gate, and with it the argument-size check.
    disallowedTools: briefing?.deniedBuiltins ?? [],
    canUseTool: makeBriefingGate({
      allowedTools: briefing?.allowedTools ?? [],
      readScopeServers: briefing?.readScopeServers ?? [],
      actionScope: briefing?.actionScope ?? null,
      exactAction: briefing?.exactAction ?? null,
      maxArgumentBytes: briefing?.maxArgumentBytes,
    }),
    // Only the reviewed connectors, and only as declared here, unless the run
    // uses the harness's own signed-in connectors (above).
    strictMcpConfig: !native,
    mcpServers: scopedMcpServers,
    // Nothing inherited. A project's settings can carry permissions and hooks,
    // and a briefing run must not gain authority from where it happens to be run.
    // A personal-app runtime is the one exception: it loads the person's own
    // user settings, as their chats do, so their configured MCP servers come
    // up with their sign-in. It gains nothing to act with: the gate above
    // denies every model tool and the runtime is never sent a turn.
    // In a trusted project folder it loads that project's settings too,
    // exactly as a chat opened there would, so a server the project
    // configures under a user-wide name resolves to the project's server.
    settingSources: isAppRuntime(briefing) ? (projectTrusted ? ["user", "project", "local"] : ["user"]) : [],
    plugins: [],
    // No dialog kinds are declared and no dialog callback is wired, so the CLI
    // emits none and every flow behind one degrades to its no-dialog behaviour.
    // This is the mechanism that makes a permission prompt or an MCP elicitation
    // unable to park a run with no human attached.
  };
}

/**
 * A `claude.ai …` connector belongs to the account: Claude Code loads it
 * itself, signed in. A copy handed over in `mcpServers` never shares that
 * sign-in and comes up `needs-auth` with no tools.
 */
export function isAccountConnector(name) {
  return typeof name === "string" && name.startsWith("claude.ai ");
}

/** A workflow run that reaches connectors (not a worktree job). */
function connectorWorkflow(briefing) {
  return Boolean(briefing?.workflow) && briefing.workflow.worktree !== true;
}

/** The granted servers of a connector workflow, by configuration key. */
export function workflowGrantedServers(briefing) {
  return connectorWorkflow(briefing) ? (briefing.workflow.servers ?? []).map((server) => server.key) : [];
}

/** The granted account connectors, which load natively and are waited for. */
export function workflowAccountServers(briefing) {
  return workflowGrantedServers(briefing).filter(isAccountConnector);
}

/**
 * Every configured server a connector workflow was not granted. Loading
 * account connectors natively means Claude Code would otherwise load all of
 * them: the gate would still refuse their tools, but their schemas would fill
 * the run's context. Denied servers never start.
 */
export function workflowDeniedServers(briefing, mcpServers = {}) {
  if (!connectorWorkflow(briefing)) return [];
  const granted = new Set(workflowGrantedServers(briefing));
  return Object.keys(mcpServers ?? {}).filter((name) => !granted.has(name));
}

/**
 * A workflow run's options: nothing inherited, the workflow gate deciding
 * every call, and only the granted servers reachable.
 *
 * `tools: []` removes every built-in tool. A deny-list cannot keep up with
 * Claude Code: tools such as `SendMessage`, `RemoteTrigger` and `CronCreate`
 * run without asking `canUseTool`, so a run that has them can message other
 * sessions or schedule work no matter what the gate says. A connector
 * workflow needs none of them; its only tools are its connectors' tools.
 *
 * Granted account connectors load natively (`strictMcpConfig: false`, no
 * copy); any other granted server is handed over as configured.
 */
function workflowOptions(briefing, mcpServers, ask) {
  const servers = briefing.workflow.servers ?? [];
  const keys = new Set(servers.map((server) => server.key));
  const native = servers.some((server) => isAccountConnector(server.key));
  return {
    permissionMode: "default",
    tools: [],
    // No `allowedTools`: anything pre-approved would skip the gate.
    disallowedTools: briefing.deniedBuiltins ?? [],
    canUseTool: makeWorkflowGate({ servers, maxArgumentBytes: briefing.maxArgumentBytes, ask }),
    strictMcpConfig: !native,
    mcpServers: Object.fromEntries(Object.entries(mcpServers).filter(([name]) => keys.has(name) && !isAccountConnector(name))),
    settingSources: [],
    plugins: [],
  };
}

/**
 * The account connectors a launch must wait for before its first turn, and
 * every server it may reach. A workflow run waits for its granted account
 * connectors; a briefing turn that loads connectors natively waits for the
 * account connectors in its scope. A direct call and a personal-app runtime
 * send no turn, so they wait for nothing here.
 */
export function connectorsToAwait(briefing) {
  if (briefing?.workflow) {
    return { granted: workflowGrantedServers(briefing), waitFor: workflowAccountServers(briefing) };
  }
  if (briefing?.nativeConnectors !== true || isDirectCall(briefing) || isAppRuntime(briefing)) {
    return { granted: [], waitFor: [] };
  }
  const granted = [...new Set([
    ...(briefing.allowedServers ?? []),
    ...(briefing.readScopeServers ?? []),
    ...(briefing.actionScope ? [briefing.actionScope.server] : []),
  ])];
  return { granted, waitFor: granted.filter(isAccountConnector) };
}

/** Is this config a briefing run? */
export function isBriefing(config) {
  return Boolean(config?.briefing);
}
