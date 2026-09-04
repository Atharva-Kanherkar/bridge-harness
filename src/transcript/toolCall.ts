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
export type ToolGlyph = "pencil" | "file-plus" | "file" | "terminal" | "search" | "globe" | "fork" | "list" | "wrench";

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
  status: ToolStatus;
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
  // Some providers only ever put the diff in the body. Taken last, and only
  // when it is unmistakably a diff.
  if (carriesPatch(source.text)) return source.text;
  if (carriesPatch(output)) return output;
  return undefined;
}

function readPath(data: Record<string, unknown>): string | undefined {
  const input = objectValue(data.input);
  const state = objectValue(data.state);
  const stateInput = objectValue(state.input);
  const direct = text(input.file_path) ?? text(input.notebook_path) ?? text(input.path)
    ?? text(data.path) ?? text(stateInput.filePath) ?? text(stateInput.file_path) ?? text(stateInput.path);
  if (direct) return direct;
  if (Array.isArray(data.changes)) {
    const first = objectValue(data.changes[0]);
    return text(first.path);
  }
  return undefined;
}

function readStatus(status: string | undefined): ToolStatus {
  if (status === "inProgress" || status === "streaming" || status === "running") return "running";
  if (status === "failed" || status === "error") return "failed";
  if (status === "completed") return "completed";
  return "idle";
}

/** The output behind a tool row: explicit output, else the item's own body. */
function readOutput(source: ToolCallSource, data: Record<string, unknown>): string | undefined {
  const state = objectValue(data.state);
  const direct = text(data.aggregatedOutput) ?? text(data.output) ?? text(state.output);
  if (direct) return direct;
  const body = source.text ?? "";
  if (!body.trim()) return undefined;
  // A title echoed back as the body is not output, it is the title again.
  if (source.title && body.trim() === source.title.trim()) return undefined;
  return body;
}

/** Read one tool call's display shape out of whatever the provider sent. */
export function readToolCall(source: ToolCallSource): ToolCallDisplay {
  const data = source.data;
  const path = readPath(data);
  const output = readOutput(source, data);
  const common = {
    path,
    output,
    additions: numberValue(data.additions),
    deletions: numberValue(data.deletions),
    durationMs: numberValue(data.durationMs),
    exitCode: readExitCode(data),
    status: readStatus(source.status),
  };
  const named = namedToolFacet(source, data);
  const command = named.command ?? (named.verb === "run" ? text(data.command) : undefined);
  return {
    ...common,
    ...named,
    path: named.path ?? path,
    command,
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
function namedToolFacet(source: ToolCallSource, data: Record<string, unknown>): {
  verb: ToolVerb; glyph: ToolGlyph; doing: string; done: string; target?: string; command?: string; path?: string;
} {
  const input = objectValue(data.input);
  const name = text(data.name);
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
    if (key === "edit" || key === "multiedit" || key === "notebookedit") return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: file ?? "file" };
    if (key === "write") return { verb: "edit", glyph: "file-plus", doing: "Writing", done: "Wrote", target: file ?? "file" };
    if (key === "grep" || key === "glob") {
      const pattern = text(input.pattern);
      return { verb: "search", glyph: "search", doing: "Searching", done: "Searched", target: pattern ? `“${pattern}”` : "files" };
    }
    if (key === "websearch") return { verb: "search", glyph: "globe", doing: "Searching the web", done: "Searched the web", target: text(input.query) };
    if (key === "webfetch") return { verb: "search", glyph: "globe", doing: "Fetching", done: "Fetched", target: text(input.url) };
    if (key === "task") return { verb: "tool", glyph: "fork", doing: "Delegating", done: "Delegated", target: text(input.description) };
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
    return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: file ?? "files" };
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
  return { verb: "tool", glyph: "wrench", doing: "Using a tool", done: "Used a tool", target: title || undefined };
}
