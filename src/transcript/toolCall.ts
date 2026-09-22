/**
 * Tool-call display data — the one place per-provider payload archaeology lives.
 *
 * One tool call arrives in as many shapes as there are providers: a Claude
 * `tool_use` block with a `name` and an `input`, a Codex ACP item with a `type`,
 * an OpenCode part with a nested `state`, a Cursor ACP `toolCall` with a `kind`.
 * The renderer used to read those apart inline, one `data.foo ?? data.bar` at a
 * time, which meant every new field the transcript wanted to show — an exit
 * code, a patch — widened an ad-hoc bag of reads spread across a component.
 *
 * Naming the shape once means the summary row, the diff, and the terminal block
 * cannot disagree about what a call was. Since #488 this runs once, at
 * ingestion: `src/transcript/codec.ts` calls `readToolCall` and the reducer
 * stamps the result onto `ConversationItem.tool`, so nothing re-derives it per
 * render and no component has to know what a provider called its fields.
 *
 * React-free on purpose, so it can be tested and reused as plain data.
 */

import type { ToolSurface } from "./events";

export type ToolVerb = "edit" | "read" | "run" | "search" | "tool";

/** Which icon the row wears. A key, not a component. */
export type ToolGlyph = "pencil" | "file-plus" | "file" | "terminal" | "search" | "globe" | "fork" | "list" | "wrench" | "brain" | "navigation";

export type ToolStatus = "running" | "completed" | "failed" | "idle";

export interface ToolCallDisplay {
  verb: ToolVerb;
  glyph: ToolGlyph;
  /** Present tense, while it runs: "Editing". */
  doing: string;
  /** Past tense, once it has: "Edited". */
  done: string;
  /** What was acted on — a basename, a search pattern, a command. */
  target?: string;
  /** The full path, when `target` is only its basename. */
  path?: string;
  /** The command as typed, for the terminal block. */
  command?: string;
  additions?: number;
  deletions?: number;
  durationMs?: number;
  /** Present only where the provider actually reports one. */
  exitCode?: number;
  /** A unified diff this call carries, to render inline. */
  patch?: string;
  /** Everything else it produced. */
  output?: string;
  /** Empty pending/running call whose action has not been named yet. */
  pendingIdentity?: boolean;
  status: ToolStatus;
  /**
   * A harness-spawned nested subagent (issue #667): the model asked its own
   * runtime to run a subagent, outside any Bridge delegation. Present only
   * when the normalized tool shape carries a task-like payload (a `Task`
   * tool name, or a collab-agent item type); never inferred from prose.
   */
  subagent?: SubagentFacet;
}

/**
 * The task payload a nested subagent call carries: what it was asked to do
 * (`prompt`), what it was called (`description`), and which agent was named
 * (`agentType`). All three are provider vocabulary, read from the tool input
 * bag — never from a harness id.
 *
 * A collab-agent lifecycle record may have none of those fields but still
 * carries a child status in `agentsStates`; `status` surfaces that lifecycle
 * so the UI does not conflate "parent tool call completed" with "child
 * subagent finished".
 */
export interface SubagentFacet {
  agentType?: string;
  description?: string;
  prompt?: string;
  status?: "running" | "completed" | "failed";
}

/**
 * Everything `readToolCall` needs, whether it came off the live stream or out
 * of the forest. Structural on purpose: the codec builds one of these before a
 * `ConversationItem` exists.
 */
export interface ToolCallSource {
  title?: string;
  text: string;
  status?: string;
  surface: ToolSurface;
  data: Record<string, unknown>;
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

/**
 * Whether a string really is a unified diff.
 *
 * Deliberately stricter than `looksLikeDiff` in `components/highlight.ts`, and
 * deliberately not a call to it: that predicate is loose on purpose, because it
 * decides how to render output nobody has classified, and it lives in a module
 * that pulls in the whole syntax-highlighting stack. Claiming `patch` is a
 * stronger statement — the transcript will show this inline, by default — so it
 * wants a hunk header or a git header, nothing inferred.
 */
function carriesPatch(text: string | undefined): boolean {
  if (!text) return false;
  const sample = text.slice(0, 4000);
  return /^@@+ /m.test(sample) || /^diff --git /m.test(sample);
}

function numberValue(value: unknown): number | undefined {
  if (typeof value === "number") return Number.isFinite(value) ? value : undefined;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function basename(path: string): string {
  const parts = path.replace(/[/\\]+$/, "").split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

/** Every place a provider has been seen to put an exit code. */
function readExitCode(data: Record<string, unknown>): number | undefined {
  const state = objectValue(data.state);
  const metadata = { ...objectValue(data.metadata), ...objectValue(state.metadata) };
  for (const candidate of [data.exitCode, data.exit_code, state.exitCode, state.exit_code, metadata.exitCode, metadata.exit_code, metadata.exit]) {
    const parsed = numberValue(candidate);
    if (parsed !== undefined) return Math.trunc(parsed);
  }
  return undefined;
}

/** The diff a file change carries, wherever the provider chose to hang it. */
function readPatch(source: ToolCallSource, data: Record<string, unknown>, output: string | undefined): string | undefined {
  const state = objectValue(data.state);
  const metadata = { ...objectValue(data.metadata), ...objectValue(state.metadata) };
  for (const candidate of [data.patch, data.diff, data.unifiedDiff, state.diff, state.patch, metadata.diff, metadata.patch]) {
    const value = text(candidate);
    if (value) return value;
  }
  // ACP file changes arrive as a list of per-file changes, each with its own
  // diff. Joined in order so a multi-file edit reads as one patch.
  if (Array.isArray(data.changes)) {
    const joined = data.changes
      .map(change => {
        const entry = objectValue(change);
        return text(entry.diff) ?? text(entry.patch) ?? text(entry.unifiedDiff);
      })
      .filter((value): value is string => !!value)
      .join("\n");
    if (joined) return joined;
  }
  const acp = acpDiffPatch(data);
  if (acp) return acp;
  // Some providers only ever put the diff in the body. Taken last, and only
  // when it is unmistakably a diff.
  if (carriesPatch(source.text)) return source.text;
  if (carriesPatch(output)) return output;
  return undefined;
}

/**
 * A unified diff synthesized from ACP's `diff` content blocks.
 *
 * ACP describes an edit as before/after text rather than as a patch, so a
 * Cursor edit reached the diff card with nothing to render. Hunk positions are
 * approximate — both sides anchor at line 1, the same compromise the Rust side
 * makes for Claude — while the `-`/`+` content is exact, which is what the
 * inline patch and the diffstat actually read.
 */
function acpDiffPatch(data: Record<string, unknown>): string | undefined {
  const blocks = objectValue(data.update).content ?? data.content;
  if (!Array.isArray(blocks)) return undefined;
  const patches = blocks
    .map(block => {
      const entry = objectValue(block);
      if (entry.type !== "diff") return undefined;
      const path = text(entry.path) ?? "file";
      const before = typeof entry.oldText === "string" ? entry.oldText : "";
      const after = typeof entry.newText === "string" ? entry.newText : "";
      if (!before && !after) return undefined;
      const removed = before ? before.split("\n").map(line => `-${line}`) : [];
      const added = after ? after.split("\n").map(line => `+${line}`) : [];
      return [
        `--- a/${path}`,
        `+++ b/${path}`,
        `@@ -1,${removed.length} +1,${added.length} @@`,
        ...removed,
        ...added,
      ].join("\n");
    })
    .filter((value): value is string => !!value);
  return patches.length ? patches.join("\n") : undefined;
}

function readPath(data: Record<string, unknown>): string | undefined {
  const input = objectValue(data.input);
  const state = objectValue(data.state);
  const stateInput = objectValue(state.input);
  const update = objectValue(data.update);
  const locations = Array.isArray(update.locations) ? update.locations : data.locations;
  const direct = text(input.file_path) ?? text(input.notebook_path) ?? text(input.path)
    ?? text(data.path) ?? text(stateInput.filePath) ?? text(stateInput.file_path) ?? text(stateInput.path);
  if (direct) return direct;
  if (Array.isArray(data.paths) && data.paths.length) {
    const first = text(data.paths[0]);
    if (first) return first;
  }
  // ACP names the files a call touched in `locations`.
  if (Array.isArray(locations) && locations.length) {
    const first = objectValue(locations[0]);
    const located = text(first.path);
    if (located) return located;
  }
  if (Array.isArray(data.changes)) {
    const first = objectValue(data.changes[0]);
    return text(first.path);
  }
  return undefined;
}

/** Paths a file-change event names after bridge-core normalization. */
function readPaths(data: Record<string, unknown>): string[] {
  if (Array.isArray(data.paths)) {
    return data.paths.filter((path): path is string => typeof path === "string" && !!path.trim());
  }
  if (Array.isArray(data.changes)) {
    return data.changes.flatMap(change => {
      const path = text(objectValue(change).path);
      return path ? [path] : [];
    });
  }
  return [];
}

/**
 * A pathless file-change row used to render as the bare word "files". Prefer
 * the files the event actually named, then the provider's own tool/type, so
 * the label still says what kind of edit it was.
 */
function fileChangeTarget(file: string | undefined, data: Record<string, unknown>): string {
  const paths = readPaths(data);
  if (paths.length > 1) {
    return `${basename(paths[0])} + ${paths.length - 1} more`;
  }
  if (file) return file;
  return text(data.tool) ?? text(data.type) ?? text(data.name) ?? "file change";
}

function readStatus(status: string | undefined): ToolStatus {
  if (status === "inProgress" || status === "streaming" || status === "running") return "running";
  if (status === "failed" || status === "error") return "failed";
  if (status === "completed") return "completed";
  return "idle";
}

/**
 * ACP hangs a tool call's output on the update's content blocks rather than on
 * a field named for it, so a Cursor tool row used to render with nothing under
 * it while the same call under Codex showed its terminal output.
 */
function acpContentText(data: Record<string, unknown>): string | undefined {
  const blocks = objectValue(data.update).content ?? data.content;
  if (!Array.isArray(blocks)) return undefined;
  const joined = blocks
    .map(block => {
      const entry = objectValue(block);
      const inner = objectValue(entry.content);
      return text(inner.text) ?? text(entry.text);
    })
    .filter((value): value is string => !!value)
    .join("\n");
  return joined || undefined;
}

/** Read a child subagent's result out of a Codex `agentsStates` bag. */
function readAgentsStatesOutput(data: Record<string, unknown>): string | undefined {
  const state = objectValue(data.state);
  const agentsStates = objectValue(data.agentsStates) ?? objectValue(state.agentsStates);
  if (!agentsStates || Object.keys(agentsStates).length === 0) return undefined;
  const threadId = text(data.threadId) ?? text(state.threadId);
  const entry = threadId ? objectValue(agentsStates[threadId]) : objectValue(Object.values(agentsStates)[0]);
  return text(entry.message) ?? text(entry.output);
}

/** The output behind a tool row: explicit output, else the item's own body. */
function readOutput(source: ToolCallSource, data: Record<string, unknown>): string | undefined {
  const state = objectValue(data.state);
  const direct = text(data.aggregatedOutput) ?? text(data.output) ?? text(state.output)
    ?? acpContentText(data)
    ?? readAgentsStatesOutput(data);
  if (direct) return direct;
  const body = source.text ?? "";
  if (!body.trim()) return undefined;
  // A title echoed back as the body is not output, it is the title again.
  if (source.title && body.trim() === source.title.trim()) return undefined;
  return body;
}

/**
 * A harness-spawned nested subagent, read off the normalized tool shape.
 *
 * Recognized provider-neutrally: a `Task`-named tool (any casing, `name` or
 * `tool`, top level or nested under the part's `state.input`), or a
 * collab-agent item type. A `dynamicToolCall`/`mcpToolCall` is only claimed
 * when it carries both a subagent-type field and a prompt field, so generic
 * MCP tools never become subagent rows. An ACP `kind: "other"` without a task
 * payload stays a generic tool row.
 */
/** Read a child subagent's lifecycle status out of a Codex `agentsStates` bag. */
function readAgentLifecycleStatus(data: Record<string, unknown>): SubagentFacet["status"] | undefined {
  const state = objectValue(data.state);
  const agentsStates = objectValue(data.agentsStates) ?? objectValue(state.agentsStates);
  if (!agentsStates || Object.keys(agentsStates).length === 0) return undefined;
  const threadId = text(data.threadId) ?? text(state.threadId);
  const entry = threadId ? objectValue(agentsStates[threadId]) : objectValue(Object.values(agentsStates)[0]);
  const status = text(entry.status);
  if (status === "inProgress" || status === "streaming" || status === "running") return "running";
  if (status === "failed" || status === "error") return "failed";
  if (status === "completed") return "completed";
  return undefined;
}

function readSubagent(source: ToolCallSource, data: Record<string, unknown>): SubagentFacet | undefined {
  const state = objectValue(data.state);
  const input = { ...objectValue(data.input), ...objectValue(state.input), ...objectValue(data.arguments) };
  const name = (text(data.name) ?? text(data.tool) ?? "").toLowerCase();
  const dataType = String(data.type ?? "");
  const isTaskName = name === "task" || name === "agent";
  const isCollabAgent = dataType === "collabAgentToolCall";
  const pickType = (bag: Record<string, unknown>): string | undefined =>
    text(bag.subagent_type) ?? text(bag.subagentType) ?? text(bag.agent) ?? text(bag.agentType) ?? text(bag.mode);
  const pickPrompt = (bag: Record<string, unknown>): string | undefined =>
    text(bag.prompt) ?? text(bag.task) ?? text(bag.instructions) ?? text(bag.query);
  const hasTypeField = pickType(input) ?? pickType(data);
  const hasPromptField = pickPrompt(input) ?? pickPrompt(data);
  const isTaskLikeDynamic = (dataType === "dynamicToolCall" || dataType === "mcpToolCall") && hasTypeField !== undefined && hasPromptField !== undefined;
  if (!isTaskName && !isCollabAgent && !isTaskLikeDynamic) return undefined;
  const toolName = text(data.name) ?? text(data.tool);
  const agentType = pickType(input) ?? pickType(data);
  const description = text(input.description) ?? text(input.taskName) ?? text(input.label) ?? text(input.summary)
    ?? text(data.description) ?? (source.title && source.title !== toolName ? source.title : undefined);
  const prompt = pickPrompt(input) ?? pickPrompt(data);
  const lifecycleStatus = readAgentLifecycleStatus(data);
  // A bare collab-agent lifecycle record (e.g. a completed `wait`) may carry
  // neither type, description, nor prompt, but it still owns a child result
  // that the transcript should surface.
  if (!agentType && !description && !prompt && !isCollabAgent) return undefined;
  return { agentType, description, prompt, status: lifecycleStatus };
}

/** Read one tool call's display shape out of whatever the provider sent. */
export function readToolCall(source: ToolCallSource): ToolCallDisplay {
  const data = source.data;
  const path = readPath(data);
  const output = readOutput(source, data);
  const subagent = readSubagent(source, data);
  const common = {
    path,
    output,
    additions: numberValue(data.additions),
    deletions: numberValue(data.deletions),
    durationMs: numberValue(data.durationMs),
    exitCode: readExitCode(data),
    status: readStatus(source.status),
    subagent,
  };
  const named = namedToolFacet(source, data);
  // Claude streams a tool call's start before the model has finished writing
  // its arguments; nothing runs until the snapshot lands. Say so, rather than
  // claiming a command is running while it is still being composed.
  if (data.phase === "preparing") {
    named.doing = "Preparing";
    named.target = text(data.name) ?? named.target;
  }
  const command = named.command ?? (named.verb === "run" ? text(data.command) : undefined);
  return {
    ...common,
    ...named,
    path: named.path ?? path,
    command,
    // Keep the call in the reduction, but do not narrate an anonymous start.
    // Actual output and terminal results remain inspectable even if the
    // provider never supplies a name or a recognized action category.
    pendingIdentity: named.pendingIdentity
      && (common.status === "running" || source.status === "pending")
      && !output,
    // Only edits show a diff inline; a read whose body happens to be a diff is
    // still just output.
    patch: named.verb === "edit" ? readPatch(source, data, output) : undefined,
  };
}

export function parseCommandTokens(command: string): string[] {
  const trimmed = command.trim();
  const stripped = trimmed.replace(/^([A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)+/, "");
  const tokens: string[] = [];
  const regex = /[^\s"']+|"([^"]*)"|'([^']*)'/g;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(stripped)) !== null) {
    tokens.push(match[1] ?? match[2] ?? match[0]);
  }
  return tokens;
}

function matchesAny(bin: string, names: string[]): boolean {
  const base = bin.split("/").pop() || bin;
  return names.includes(base);
}

function isFlag(arg: string): boolean {
  return /^--?[a-zA-Z0-9]/.test(arg);
}

function classifySingleCommand(cmd: string): {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
} | null {
  let clean = cmd.trim().replace(/^([A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)+/, "");
  clean = clean.replace(/^(?:builtin|command|sudo)\s+/, "");
  const tokens = parseCommandTokens(clean);
  if (!tokens.length) return null;

  const bin = tokens[0].toLowerCase();
  const args = tokens.slice(1);

  if (matchesAny(bin, ["cat", "head", "tail", "less", "more", "bat"])) {
    let idx = 0;
    while (idx < args.length) {
      if (args[idx] === "-n" || args[idx] === "-c") {
        idx += 2;
      } else if (isFlag(args[idx])) {
        idx += 1;
      } else {
        break;
      }
    }
    const filePath = args[idx];
    const target = filePath ? (filePath.split("/").pop() || filePath) : undefined;
    return {
      verb: "read",
      glyph: "file",
      doing: "Reading",
      done: "Read",
      target: target ?? filePath,
      path: filePath,
    };
  }

  if (matchesAny(bin, ["ls", "dir", "tree"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const dirPath = nonFlags[0];
    return {
      verb: "read",
      glyph: "file",
      doing: "Listing",
      done: "Listed",
      target: dirPath ? (dirPath.split("/").pop() || dirPath) : "directory",
      path: dirPath,
    };
  }

  if (matchesAny(bin, ["grep", "egrep", "fgrep", "rg", "ag", "ack"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const pattern = nonFlags[0];
    const filePath = nonFlags[1];
    return {
      verb: "search",
      glyph: "search",
      doing: "Searching",
      done: "Searched",
      target: pattern ? `“${pattern}”` : "files",
      path: filePath,
    };
  }

  if (matchesAny(bin, ["find", "fd", "locate", "which", "whereis", "wc", "stat", "file"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const target = nonFlags[0];
    return {
      verb: "search",
      glyph: "search",
      doing: "Searching",
      done: "Searched",
      target: target ? `“${target}”` : "files",
      path: target,
    };
  }

  if (bin === "git") {
    let subIdx = 0;
    while (subIdx < args.length && isFlag(args[subIdx])) {
      if (args[subIdx] === "-C" || args[subIdx] === "-c") subIdx += 2;
      else subIdx += 1;
    }
    const sub = args[subIdx]?.toLowerCase();
    if (!sub) return null;

    if (sub === "status") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Checking",
        done: "Checked",
        target: "git status",
      };
    }
    if (sub === "diff") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Inspecting",
        done: "Inspected",
        target: "git diff",
      };
    }
    if (sub === "log") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Viewing",
        done: "Viewed",
        target: "git log",
      };
    }
    if (sub === "show") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Inspecting",
        done: "Inspected",
        target: "git show",
      };
    }
    if (sub === "branch" || sub === "tag" || sub === "remote" || sub === "describe") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Checking",
        done: "Checked",
        target: `git ${sub}`,
      };
    }
    return null;
  }

  return null;
}

export function hasUnquotedRedirect(command: string): boolean {
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < command.length; i++) {
    const ch = command[i];
    if (ch === "\\" && !inSingle) {
      i++;
      continue;
    }
    if (ch === "'" && !inDouble) {
      inSingle = !inSingle;
      continue;
    }
    if (ch === '"' && !inSingle) {
      inDouble = !inDouble;
      continue;
    }
    if (!inSingle && !inDouble && ch === ">") {
      return true;
    }
  }
  return false;
}

export function classifyExploratoryCommand(rawCommand: string): {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
} | null {
  const trimmed = rawCommand.trim();
  if (!trimmed) return null;
  if (hasUnquotedRedirect(trimmed)) return null;

  const parts = trimmed.split(/\s*(?:&&|;|\|\|)\s*/).filter(Boolean);
  if (parts.length > 1) {
    const classifiedParts = parts.map(classifySingleCommand);
    if (classifiedParts.some(c => c === null)) return null;
    const first = classifiedParts[0]!;
    return {
      verb: first.verb,
      glyph: first.glyph,
      doing: "Exploring",
      done: "Explored",
      target: trimmed,
    };
  }

  if (trimmed.includes("|")) {
    const pipeParts = trimmed.split(/\s*\|\s*/).filter(Boolean);
    const classifiedPipe = pipeParts.map(classifySingleCommand);
    if (classifiedPipe.some(c => c === null)) return null;
    const first = classifiedPipe[0]!;
    return {
      verb: first.verb,
      glyph: first.glyph,
      doing: first.doing,
      done: first.done,
      target: trimmed,
      path: first.path,
    };
  }

  return classifySingleCommand(trimmed);
}

/** Verb, glyph, wording and target — the half of the shape that depends on
 *  *which* tool ran rather than on how it went. */
/**
 * The verbs ACP names its tool categories with. Cursor and every other ACP
 * agent send one of these on `data.kind`; without them the transcript could
 * only guess from the title, which is how a Cursor read used to render as an
 * anonymous "Using a tool".
 */
const ACP_TOOL_KINDS: Record<string, { verb: ToolVerb; glyph: ToolGlyph; doing: string; done: string }> = {
  think: { verb: "tool", glyph: "brain", doing: "Thinking", done: "Thought" },
  switch_mode: { verb: "tool", glyph: "navigation", doing: "Switching mode", done: "Switched mode" },
  read: { verb: "read", glyph: "file", doing: "Reading", done: "Read" },
  edit: { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited" },
  delete: { verb: "edit", glyph: "pencil", doing: "Deleting", done: "Deleted" },
  move: { verb: "edit", glyph: "pencil", doing: "Moving", done: "Moved" },
  search: { verb: "search", glyph: "search", doing: "Searching", done: "Searched" },
  execute: { verb: "run", glyph: "terminal", doing: "Running", done: "Ran" },
  fetch: { verb: "search", glyph: "globe", doing: "Fetching", done: "Fetched" },
};

function namedToolFacet(source: ToolCallSource, data: Record<string, unknown>): {
  verb: ToolVerb; glyph: ToolGlyph; doing: string; done: string; target?: string; command?: string; path?: string; pendingIdentity?: boolean;
} {
  // Claude puts the arguments on `input`; OpenCode nests them under the part's
  // `state`. Merged so the branches below can read one bag.
  const state = objectValue(data.state);
  const input = { ...objectValue(data.input), ...objectValue(state.input) };
  // Claude names the tool `name`, OpenCode names it `tool`. Same question.
  const name = text(data.name) ?? text(data.tool);
  const dataType = String(data.type ?? "");
  const title = source.title ?? "";
  const path = readPath(data);
  const file = path ? basename(path) : undefined;

  if (name) {
    const key = name.toLowerCase();
    if (key === "bash" || key === "shell") {
      const command = text(input.command) ?? text(data.command);
      const exploratory = command ? classifyExploratoryCommand(command) : null;
      if (exploratory) {
        return { ...exploratory, command };
      }
      return { verb: "run", glyph: "terminal", doing: "Running", done: "Ran", target: command ?? (title || "command"), command };
    }
    if (key === "read") return { verb: "read", glyph: "file", doing: "Reading", done: "Read", target: file ?? "file" };
    if (key === "edit" || key === "multiedit" || key === "notebookedit") return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: fileChangeTarget(file, data) };
    if (key === "write") return { verb: "edit", glyph: "file-plus", doing: "Writing", done: "Wrote", target: fileChangeTarget(file, data) };
    if (key === "patch" || key === "apply_patch") return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: fileChangeTarget(file, data) };
    if (key === "grep" || key === "glob") {
      const pattern = text(input.pattern);
      return { verb: "search", glyph: "search", doing: "Searching", done: "Searched", target: pattern ? `“${pattern}”` : "files" };
    }
    if (key === "websearch") return { verb: "search", glyph: "globe", doing: "Searching the web", done: "Searched the web", target: text(input.query) };
    if (key === "webfetch") return { verb: "search", glyph: "globe", doing: "Fetching", done: "Fetched", target: text(input.url) };
    if (key === "task" || key === "agent") return { verb: "tool", glyph: "fork", doing: "Delegating", done: "Delegated", target: text(input.description) };
    if (key === "todowrite") return { verb: "tool", glyph: "list", doing: "Updating tasks", done: "Updated tasks" };
    if (key.startsWith("mcp__")) {
      const parts = name.replace(/^mcp__/, "").split("__");
      const server = parts[0] ?? name;
      const tool = parts.slice(1).join(" ").replaceAll("_", " ") || name;
      return { verb: "tool", glyph: "wrench", doing: `Using ${server}`, done: `Used ${server}`, target: tool };
    }
    return { verb: "tool", glyph: "wrench", doing: `Using ${name}`, done: `Used ${name}`, target: title || undefined };
  }

  // Codex- and OpenCode-shaped items, identified by their item type.
  if (source.surface === "diff" || dataType.includes("patch") || dataType.includes("fileChange")) {
    return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: fileChangeTarget(file, data) };
  }
  if (dataType === "readFile" || /^read /i.test(title)) {
    const named = file ?? (title.replace(/^read /i, "") || undefined);
    return { verb: "read", glyph: "file", doing: "Reading", done: "Read", target: named ?? "file" };
  }
  if (dataType === "commandExecution" || data.command) {
    const command = text(data.command) ?? (title || undefined);
    const exploratory = command ? classifyExploratoryCommand(command) : null;
    if (exploratory) {
      return { ...exploratory, command };
    }
    return { verb: "run", glyph: "terminal", doing: "Running", done: "Ran", target: command ?? "command", command };
  }
  if (dataType === "webSearch") {
    return { verb: "search", glyph: "globe", doing: "Searching the web", done: "Searched the web", target: title || undefined };
  }
  // ACP-shaped items, identified by the category the protocol itself names.
  const acp = ACP_TOOL_KINDS[String(data.kind ?? "")];
  if (acp) {
    const command = acp.verb === "run" ? text(data.command) ?? (title || undefined) : undefined;
    const exploratory = command ? classifyExploratoryCommand(command) : null;
    if (exploratory) return { ...exploratory, command };
    return { ...acp, target: acp.verb === "run" ? command ?? "command" : file ?? (title || undefined), command };
  }
  // Provider-neutral fallback: preserve the action wording, with an explicit
  // state cue that does not depend on the tense chosen by the provider.
  const action = text(title)?.trim();
  if (action) return { verb: "tool", glyph: "wrench", doing: `Running: ${action}`, done: `Finished: ${action}` };
  return { verb: "tool", glyph: "wrench", doing: "Using a tool", done: "Used a tool", pendingIdentity: true };
}
