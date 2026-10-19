/**
 * A shell command read apart into the steps it chains, in words.
 *
 * A model's command is written for a shell, not for a reader:
 * `ls tools/ 2>/dev/null; git -C tools/vllm log -1; cat .gitignore | head -50`
 * is three things it wanted to know, wrapped in the plumbing that keeps the
 * output short. This module undoes the plumbing: it unwraps the harness's own
 * `zsh -lc "…"`, splits the script on its operators (never inside quotes,
 * substitutions or heredocs), names each pipeline as one step, turns the noise
 * (`2>/dev/null`, `| head -50`) into notes, and says what each step does to the
 * machine: reads, writes files, or runs code.
 *
 * Deterministic on purpose. Nothing here asks a model to summarise, and a
 * program it does not know is `exec`, never `read`: the effect chip is a claim
 * a reader may lean on.
 *
 * React-free, so it tests as data, and cached per command string because the
 * reducer re-reads a tool call on every output delta.
 */

import type { ToolGlyph, ToolVerb } from "./toolCall";

/** What a command does to the machine, weakest first. */
export type CommandEffect = "read" | "write" | "exec";

const EFFECT_RANK: Record<CommandEffect, number> = { read: 0, write: 1, exec: 2 };

/** The chip a row wears for its command's effect. */
export const EFFECT_LABEL: Record<CommandEffect, string> = {
  read: "read-only",
  write: "changes files",
  exec: "runs code",
};

/** One pipeline of a command, named. */
export interface CommandStep {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
  effect: CommandEffect;
  /** The plumbing, in words: "errors hidden", "first 50 lines", "in tools/vllm". */
  notes: string[];
  /** The pipeline as typed. */
  text: string;
}

export interface CommandReading {
  /** The script once the harness's shell wrapper is gone. */
  script: string;
  steps: CommandStep[];
  /** The strongest effect any step has. */
  effect: CommandEffect;
}

/* ── Unwrapping ─────────────────────────────────────────────────────────── */

const WRAPPER = /^\s*(?:\S*\/)?(?:ba|z|da|k)?sh\s+-[a-zA-Z]*c\s+/;

/**
 * The script inside a harness's shell wrapper. Codex runs nearly every command
 * as `/bin/zsh -lc "…"`; the wrapper is how it executes, not what the model
 * asked for. Anything that is not exactly one quoted script after `-c` (a
 * script file, trailing positional arguments) is returned as typed.
 */
export function unwrapShell(command: string): string {
  const head = WRAPPER.exec(command);
  if (!head) return command;
  const rest = command.slice(head[0].length).trim();
  if (rest.startsWith('"')) {
    let out = "";
    let index = 1;
    for (; index < rest.length; index++) {
      const ch = rest[index];
      if (ch === "\\" && index + 1 < rest.length && `"\\$\`\n`.includes(rest[index + 1])) {
        if (rest[index + 1] !== "\n") out += rest[index + 1];
        index++;
        continue;
      }
      if (ch === '"') break;
      out += ch;
    }
    return index === rest.length - 1 ? out : command;
  }
  if (rest.startsWith("'")) {
    // A single-quoted script splices its own quotes as '"'"' or '\''.
    let out = "";
    let index = 0;
    while (index < rest.length) {
      if (rest[index] === "'") {
        const end = rest.indexOf("'", index + 1);
        if (end < 0) return command;
        out += rest.slice(index + 1, end);
        index = end + 1;
      } else if (rest.startsWith(`"'"`, index)) {
        out += "'";
        index += 3;
      } else if (rest.startsWith("\\'", index)) {
        out += "'";
        index += 2;
      } else {
        return command;
      }
    }
    return out;
  }
  return /^\S+$/.test(rest) ? rest : command;
}

/* ── Splitting ──────────────────────────────────────────────────────────── */

type Operator = ";" | "&&" | "||" | "|" | "&" | "\n";

interface Piece {
  text: string;
  /** The operator that ends this piece, if any. */
  op: Operator | null;
  /** Bodies of unquoted heredocs opened here: the shell expands `$(…)` in them. */
  bodies?: string[];
}

/**
 * The script cut at its top-level operators. Quotes, `$(…)`, backticks,
 * subshells and heredoc bodies are opaque: `grep "a;b" f` is one piece.
 */
export function splitShell(script: string): Piece[] {
  const pieces: Piece[] = [];
  let buffer = "";
  let single = false;
  let double = false;
  let backtick = false;
  let depth = 0;
  const heredocs: Array<{ tag: string; strip: boolean; literal: boolean }> = [];
  let bodies: string[] = [];
  const push = (op: Operator | null) => {
    const text = buffer.trim();
    if (text) pieces.push({ text, op, ...(bodies.length ? { bodies } : {}) });
    else {
      const last = pieces[pieces.length - 1];
      if (op && last && last.op === null) last.op = op;
      if (bodies.length && last) last.bodies = [...(last.bodies ?? []), ...bodies];
    }
    buffer = "";
    bodies = [];
  };
  for (let index = 0; index < script.length; index++) {
    const ch = script[index];
    if (single) {
      buffer += ch;
      if (ch === "'") single = false;
      continue;
    }
    if (ch === "\\") {
      buffer += ch + (script[index + 1] ?? "");
      index++;
      continue;
    }
    if (double) {
      buffer += ch;
      if (ch === '"') double = false;
      continue;
    }
    if (ch === "'") { single = true; buffer += ch; continue; }
    if (ch === '"') { double = true; buffer += ch; continue; }
    if (ch === "`") { backtick = !backtick; buffer += ch; continue; }
    if (backtick) { buffer += ch; continue; }
    if (ch === "(") { depth++; buffer += ch; continue; }
    if (ch === ")") { depth = Math.max(0, depth - 1); buffer += ch; continue; }
    if (depth > 0) { buffer += ch; continue; }
    if (ch === "<" && script[index + 1] === "<" && script[index + 2] !== "<") {
      const heredoc = /^<<(-?)\s*(['"]?)([A-Za-z_][\w-]*)\2/.exec(script.slice(index));
      if (heredoc) {
        heredocs.push({ tag: heredoc[3], strip: heredoc[1] === "-", literal: !!heredoc[2] });
        buffer += heredoc[0];
        index += heredoc[0].length - 1;
        continue;
      }
    }
    if (ch === "\n") {
      if (heredocs.length) {
        // The body belongs to the command that opened it; skip to its end tag.
        let cursor = index + 1;
        for (const doc of heredocs) {
          const lines: string[] = [];
          while (cursor < script.length) {
            const newline = script.indexOf("\n", cursor);
            const end = newline < 0 ? script.length : newline;
            const line = script.slice(cursor, end);
            cursor = end + 1;
            if ((doc.strip ? line.replace(/^\t+/, "") : line).trim() === doc.tag) break;
            lines.push(line);
          }
          // A quoted delimiter (`<<'EOF'`) makes the body literal text.
          if (!doc.literal && lines.length) bodies.push(lines.join("\n"));
        }
        heredocs.length = 0;
        index = cursor - 1;
      }
      push("\n");
      continue;
    }
    if (ch === ";") { push(";"); continue; }
    if (ch === "&" && script[index + 1] === "&") { push("&&"); index++; continue; }
    if (ch === "|" && script[index + 1] === "|") { push("||"); index++; continue; }
    if (ch === "|") { push("|"); continue; }
    if (ch === "&" && script[index + 1] !== ">" && script[index - 1] !== ">") { push("&"); continue; }
    buffer += ch;
  }
  push(null);
  return pieces;
}

/* ── Nested commands ────────────────────────────────────────────────────── */

/** Index just past the `)` that closes a `(` opened before `from`, or -1. */
function closingParen(text: string, from: number): number {
  let depth = 1;
  let single = false;
  let double = false;
  for (let index = from; index < text.length; index++) {
    const ch = text[index];
    if (single) { if (ch === "'") single = false; continue; }
    if (ch === "\\") { index++; continue; }
    if (double) { if (ch === '"') double = false; continue; }
    if (ch === "'") single = true;
    else if (ch === '"') double = true;
    else if (ch === "(") depth++;
    else if (ch === ")" && --depth === 0) return index + 1;
  }
  return -1;
}

/**
 * The commands a piece of shell runs inside itself: `$(…)`, backticks, and
 * `<(…)`/`>(…)`, wherever the shell expands them (not inside single quotes;
 * process substitution not inside double quotes either). Arithmetic `$((…))`
 * is not a command. `null` when one is left open: whatever it is, it runs.
 */
export function substitutions(text: string): string[] | null {
  const found: string[] = [];
  let single = false;
  let double = false;
  for (let index = 0; index < text.length; index++) {
    const ch = text[index];
    if (single) { if (ch === "'") single = false; continue; }
    if (ch === "\\") { index++; continue; }
    if (ch === "'" && !double) { single = true; continue; }
    if (ch === '"') { double = !double; continue; }
    if (ch === "$" && text[index + 1] === "(") {
      if (text[index + 2] === "(") { index += 2; continue; }
      const end = closingParen(text, index + 2);
      if (end < 0) return null;
      found.push(text.slice(index + 2, end - 1));
      index = end - 1;
      continue;
    }
    if (ch === "`") {
      let end = index + 1;
      while (end < text.length && text[end] !== "`") end += text[end] === "\\" ? 2 : 1;
      if (end >= text.length) return null;
      found.push(text.slice(index + 1, end));
      index = end;
      continue;
    }
    if ((ch === "<" || ch === ">") && text[index + 1] === "(" && !double) {
      const end = closingParen(text, index + 2);
      if (end < 0) return null;
      found.push(text.slice(index + 2, end - 1));
      index = end - 1;
    }
  }
  return found;
}

/** Whether running these texts runs anything beyond reads. */
function runsNestedCode(texts: string[]): boolean {
  for (const text of texts) {
    const nested = substitutions(text);
    if (nested === null) return true;
    if (nested.some(body => readCommand(body).effect !== "read")) return true;
  }
  return false;
}

/** Env prefixes that load or run code before the program starts. */
const RUNNING_ENV = /^(LD_PRELOAD|LD_LIBRARY_PATH|DYLD_[A-Z_]+|GIT_EXTERNAL_DIFF|GIT_PAGER|PAGER|GIT_SSH|GIT_SSH_COMMAND|GIT_ASKPASS|SSH_ASKPASS|GIT_EDITOR|EDITOR|VISUAL|BASH_ENV|ENV|PROMPT_COMMAND|PERL5OPT|PYTHONSTARTUP|NODE_OPTIONS|GIT_CONFIG_PARAMETERS|GIT_CONFIG_COUNT|GIT_PROXY_COMMAND)=/;

/* ── Lexing one command ─────────────────────────────────────────────────── */

interface Redirect {
  op: string;
  target?: string;
}

interface Lexed {
  words: string[];
  redirects: Redirect[];
}

/** Words with their quotes removed, and the redirections pulled out of them. */
function lex(stage: string): Lexed {
  const words: string[] = [];
  const redirects: Redirect[] = [];
  let current = "";
  let has = false;
  let quoted = false;
  let pending: Redirect | null = null;
  const flush = () => {
    if (has) {
      if (pending) { pending.target = current; pending = null; } else words.push(current);
    }
    current = "";
    has = false;
    quoted = false;
  };
  for (let index = 0; index < stage.length; index++) {
    const ch = stage[index];
    if (ch === "'" || ch === '"') {
      const end = ch === "'" ? stage.indexOf("'", index + 1) : closingDouble(stage, index + 1);
      const body = stage.slice(index + 1, end < 0 ? stage.length : end);
      current += ch === '"' ? body.replace(/\\(["\\$`])/g, "$1") : body;
      has = true;
      quoted = true;
      index = end < 0 ? stage.length : end;
      continue;
    }
    if (ch === "\\" && index + 1 < stage.length) {
      current += stage[++index];
      has = true;
      continue;
    }
    if (/\s/.test(ch)) { flush(); continue; }
    if (ch === ">" || ch === "<") {
      let op = "";
      if (has && !quoted && /^(\d+|&)$/.test(current)) {
        op = current;
        current = "";
        has = false;
      } else {
        flush();
      }
      op += ch;
      while (stage[index + 1] === ch && op.replace(/^[\d&]*/, "").length < 3) op += stage[++index];
      const dup = /^&(\d+|-)?/.exec(stage.slice(index + 1));
      if (dup && ch === ">") {
        op += dup[0];
        index += dup[0].length;
        if (dup[1]) { redirects.push({ op }); continue; }
      }
      const redirect: Redirect = { op };
      redirects.push(redirect);
      pending = redirect;
      continue;
    }
    current += ch;
    has = true;
  }
  flush();
  return { words, redirects };
}

function closingDouble(text: string, from: number): number {
  for (let index = from; index < text.length; index++) {
    if (text[index] === "\\") { index++; continue; }
    if (text[index] === '"') return index;
  }
  return -1;
}

/* ── Naming a step ──────────────────────────────────────────────────────── */

interface Phrase {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
  effect: CommandEffect;
  notes?: string[];
}

function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.split("/").pop() || path;
}

function shortPath(path: string): string {
  if (path.length <= 32) return path;
  const parts = path.replace(/\/+$/, "").split("/");
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : path;
}

function clip(text: string, max = 40): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function isFlag(word: string): boolean {
  return /^--?[A-Za-z0-9]/.test(word);
}

/** Positional arguments, skipping each flag and, for `valued` flags, its value. */
function positionals(args: string[], valued: Set<string> = new Set()): string[] {
  const out: string[] = [];
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (arg === "--") { out.push(...args.slice(index + 1)); break; }
    if (isFlag(arg)) {
      if (!arg.includes("=") && valued.has(arg)) index++;
      continue;
    }
    out.push(arg);
  }
  return out;
}

function flagValue(args: string[], names: string[]): string | undefined {
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    for (const name of names) {
      if (arg === name) return args[index + 1];
      if (arg.startsWith(`${name}=`)) return arg.slice(name.length + 1);
      // Short flags take their value glued on: `-n20`.
      if (name.length === 2 && arg.startsWith(name) && arg.length > 2 && !name.startsWith("--")) return arg.slice(2);
    }
  }
  return undefined;
}

const read = (doing: string, done: string, target?: string, extra: Partial<Phrase> = {}): Phrase =>
  ({ verb: "read", glyph: "file", doing, done, target, effect: "read", ...extra });
/** A read-only step that is not a file read: it counts as a command, not a file. */
const look = (doing: string, done: string, target?: string, extra: Partial<Phrase> = {}): Phrase =>
  ({ verb: "run", glyph: "terminal", doing, done, target, effect: "read", ...extra });
const search = (target: string, path?: string): Phrase =>
  ({ verb: "search", glyph: "search", doing: "Searching", done: "Searched", target, path, effect: "read" });
const write = (doing: string, done: string, target?: string, extra: Partial<Phrase> = {}): Phrase =>
  ({ verb: "run", glyph: "terminal", doing, done, target, effect: "write", ...extra });
const exec = (doing: string, done: string, target?: string, extra: Partial<Phrase> = {}): Phrase =>
  ({ verb: "run", glyph: "terminal", doing, done, target, effect: "exec", ...extra });

function files(paths: string[]): { target?: string; path?: string } {
  if (!paths.length) return {};
  const first = paths[0];
  const more = paths.length - 1;
  return { target: more ? `${basename(first)} and ${more} more` : basename(first), path: first };
}

const GREP_VALUED = new Set(["-e", "-f", "-g", "--glob", "-t", "--type", "-T", "--type-not", "-A", "-B", "-C", "-m", "--max-count", "-M", "--max-columns", "-j", "--threads", "--include", "--exclude", "--exclude-dir", "-d", "--max-depth"]);

function grepPhrase(args: string[]): Phrase {
  const explicit = flagValue(args, ["-e", "--regexp"]);
  const rest = positionals(args, GREP_VALUED);
  const pattern = explicit ?? rest[0];
  const path = explicit !== undefined ? rest[0] : rest[1];
  return search(pattern ? `“${clip(pattern, 48)}”` : "files", path);
}

function linesNote(script: string | undefined): string | undefined {
  if (!script) return undefined;
  const range = /^(\d+),(\d+|\$)p$/.exec(script.trim());
  if (range) return range[2] === "$" ? `from line ${range[1]}` : `lines ${range[1]} to ${range[2]}`;
  const one = /^(\d+)p$/.exec(script.trim());
  return one ? `line ${one[1]}` : undefined;
}

function headNote(bin: string, args: string[]): string {
  const bytes = flagValue(args, ["-c", "--bytes"]);
  if (bytes) return bin === "head" ? `first ${bytes.replace(/^\+/, "")} bytes` : `last ${bytes} bytes`;
  const glued = args.find(arg => /^-\d+$/.test(arg));
  const lines = flagValue(args, ["-n", "--lines"]) ?? glued?.slice(1) ?? "10";
  if (bin === "tail" && lines.startsWith("+")) return `from line ${lines.slice(1)}`;
  const n = lines.replace(/^[+-]/, "");
  return bin === "head" ? `first ${n} line${n === "1" ? "" : "s"}` : `last ${n} line${n === "1" ? "" : "s"}`;
}

const GH_VALUED = new Set(["-R", "--repo", "--json", "-q", "--jq", "-t", "--template", "-L", "--limit", "-s", "--state", "-l", "--label", "-a", "--assignee", "-A", "--author", "-B", "--base", "-H", "--head", "-b", "--body", "-F", "--body-file", "--title", "-X", "--method", "-f", "--field", "--raw-field", "--header", "--input", "-w", "--workflow", "-j", "--job", "-c", "--commit", "-m", "--milestone", "-p", "--project", "-r", "--reviewer", "-e", "--search", "-S"]);

function ghPhrase(args: string[]): Phrase {
  const github = (phrase: Phrase): Phrase => ({ ...phrase, glyph: "github" });
  const rest = positionals(args, GH_VALUED);
  const [noun, verb, subject] = rest;
  const pr = subject ? `PR #${subject.replace(/^#/, "")}` : "the current PR";
  const issue = subject ? `issue #${subject.replace(/^#/, "")}` : "an issue";
  if (noun === "api") {
    const method = (flagValue(args, ["-X", "--method"]) ?? "").toUpperCase();
    const fields = args.some(arg => /^(-f|-F|--field|--raw-field|--input)(=|$)/.test(arg));
    const reads = method ? method === "GET" : !fields;
    const target = `GitHub API ${clip(verb ?? "", 48)}`.trim();
    return github(reads ? look("Calling", "Called", target) : exec("Calling", "Called", target));
  }
  if (noun === "auth" && verb === "status") return github(look("Checking", "Checked", "GitHub sign-in"));
  if (noun === "pr") {
    switch (verb) {
      case "view": return github(look("Reading", "Read", pr));
      case "diff": return github(look("Reading", "Read", `${pr} diff`));
      case "checks": return github(look("Checking", "Checked", `CI on ${pr}`));
      case "list": return github(look("Listing", "Listed", "pull requests"));
      case "status": return github(look("Checking", "Checked", "pull request status"));
      case "create": return github(exec("Opening", "Opened", "a pull request"));
      case "merge": return github(exec("Merging", "Merged", pr));
      case "comment": return github(exec("Commenting", "Commented", `on ${pr}`));
      case "edit": return github(exec("Editing", "Edited", pr));
      case "close": return github(exec("Closing", "Closed", pr));
      case "review": return github(exec("Reviewing", "Reviewed", pr));
      case "checkout": return github(write("Checking out", "Checked out", pr));
    }
  }
  if (noun === "issue") {
    switch (verb) {
      case "view": return github(look("Reading", "Read", issue));
      case "list": return github(look("Listing", "Listed", "issues"));
      case "create": return github(exec("Opening", "Opened", "an issue"));
      case "comment": return github(exec("Commenting", "Commented", `on ${issue}`));
      case "edit": return github(exec("Editing", "Edited", issue));
      case "close": return github(exec("Closing", "Closed", issue));
    }
  }
  if (noun === "run") {
    const run = subject ? `workflow run ${subject}` : "the latest workflow run";
    switch (verb) {
      case "view": return github(look("Reading", "Read", run));
      case "list": return github(look("Listing", "Listed", "workflow runs"));
      case "watch": return github(look("Watching", "Watched", run));
      case "rerun": return github(exec("Rerunning", "Reran", run));
    }
  }
  const reads = verb !== undefined && ["view", "list", "diff", "checks", "status"].includes(verb);
  const target = ["gh", noun, verb].filter(Boolean).join(" ");
  return github(reads ? look("Running", "Ran", target) : exec("Running", "Ran", target));
}

function gitPhrase(args: string[]): Phrase {
  const notes: string[] = [];
  let index = 0;
  while (index < args.length && isFlag(args[index])) {
    if (args[index] === "-C") {
      if (args[index + 1]) notes.push(`in ${shortPath(args[index + 1])}`);
      index += 2;
    } else if (args[index] === "-c") {
      index += 2;
    } else {
      index += 1;
    }
  }
  const sub = args[index]?.toLowerCase();
  const rest = args.slice(index + 1);
  const free = positionals(rest, new Set(["-m", "--message", "-b", "-B", "-c", "-C", "--format", "--pretty", "-n", "--max-count", "--author", "--since", "--until", "-L", "-U", "--unified"]));
  const git = (phrase: Phrase): Phrase => ({ ...phrase, glyph: "git", notes: [...notes, ...(phrase.notes ?? [])] });
  const has = (...flags: string[]) => rest.some(arg => flags.includes(arg));
  switch (sub) {
    case undefined: return git(exec("Running", "Ran", "git"));
    case "status": return git(read("Checking", "Checked", "git status"));
    case "diff": return git(read("Inspecting", "Inspected", "git diff"));
    case "log": return git(read("Viewing", "Viewed", "git log"));
    case "show": return git(read("Inspecting", "Inspected", "git show"));
    case "describe": return git(read("Checking", "Checked", "git describe"));
    case "rev-parse": case "ls-files": case "ls-tree": case "cat-file": case "shortlog": case "reflog": case "merge-base":
      return git(read("Checking", "Checked", `git ${sub}`));
    case "blame": return git(read("Reading", "Read", `blame of ${free[0] ? basename(free[0]) : "file"}`, { path: free[0] }));
    case "grep": return { ...grepPhrase(rest), glyph: "git", notes };
    case "branch":
      if (has("-d", "-D", "--delete", "-m", "-M", "--move", "-c", "-C", "--copy") || (free.length && !has("-a", "-r", "--list", "--show-current", "--contains", "-v", "-vv")))
        return git(write("Changing", "Changed", "branches"));
      return git(read("Checking", "Checked", "git branch"));
    case "tag": return git(free.length && !has("-l", "--list") ? write("Tagging", "Tagged", free[0]) : read("Checking", "Checked", "git tag"));
    case "remote": return git(["add", "remove", "rm", "rename", "set-url"].includes(free[0] ?? "") ? write("Changing", "Changed", "remotes") : read("Checking", "Checked", "git remote"));
    case "config": return git(has("--get", "--list", "-l", "--get-all") || free.length <= 1 ? read("Checking", "Checked", "git config") : write("Setting", "Set", `git config ${free[0]}`));
    case "stash": return git(free[0] === "list" || free[0] === "show" ? read("Checking", "Checked", "the stash") : write("Stashing", "Stashed", free[0] === "pop" || free[0] === "apply" ? "changes back" : "changes"));
    case "worktree":
      if (free[0] === "list") return git(read("Listing", "Listed", "worktrees"));
      return git(write(free[0] === "remove" ? "Removing" : "Adding", free[0] === "remove" ? "Removed" : "Added", `worktree ${free[1] ? shortPath(free[1]) : ""}`.trim()));
    case "add": return git(write("Staging", "Staged", free.length && free[0] !== "." && free[0] !== "-A" ? files(free).target : "changes"));
    case "commit": return git(write("Committing", "Committed", "changes"));
    case "checkout": case "switch": {
      if (rest.includes("--") || (free.length > 1 && sub === "checkout")) return git(write("Restoring", "Restored", files(free.filter(path => path !== "--")).target ?? "files"));
      const branch = flagValue(rest, ["-b", "-B", "-c", "-C"]);
      return git(write("Switching to", "Switched to", branch ? `new branch ${branch}` : free[0] ?? "a branch"));
    }
    case "restore": return git(write("Restoring", "Restored", files(free).target ?? "files"));
    case "reset": return git(write("Resetting", "Reset", free[0] ? `to ${free[0]}` : "the index", has("--hard") ? { notes: ["discards changes"] } : {}));
    case "merge": return git(write("Merging", "Merged", free[0] ?? "a branch"));
    case "rebase": return git(write("Rebasing", "Rebased", free[0] ? `onto ${free[0]}` : "the branch"));
    case "cherry-pick": return git(write("Cherry-picking", "Cherry-picked", free[0] ?? "a commit"));
    case "apply": case "am": return git(write("Applying", "Applied", "a patch"));
    case "rm": return git(write("Removing", "Removed", files(free).target ?? "files"));
    case "mv": return git(write("Moving", "Moved", free.length >= 2 ? `${basename(free[0])} to ${basename(free[1])}` : "files"));
    case "clean": return git(write("Cleaning", "Cleaned", "untracked files"));
    case "push": return git(exec("Pushing", "Pushed", free[1] ?? "the branch"));
    case "pull": return git(exec("Pulling", "Pulled", free[1] ?? "the branch"));
    case "fetch": return git(exec("Fetching", "Fetched", free[0] ?? "the remote"));
    case "clone": return git(exec("Cloning", "Cloned", free[0] ? basename(free[0]).replace(/\.git$/, "") : "a repository"));
    default: return git(exec("Running", "Ran", `git ${sub}`));
  }
}

/** Interpreters and runners: name the script, not the binary's path. */
function runPhrase(bin: string, args: string[]): Phrase {
  const shown = args.slice(0, 2).map(arg => (arg.includes("/") && !isFlag(arg) ? basename(arg) : arg));
  return exec("Running", "Ran", clip([basename(bin), ...shown].join(" "), 56));
}

const ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=/;
const PREFIXES = new Set(["sudo", "time", "nohup", "exec", "builtin", "nice"]);

/** The program and its arguments once env assignments and launch prefixes are gone. */
function program(words: string[]): string[] {
  let rest = words;
  for (;;) {
    while (rest.length && ASSIGNMENT.test(rest[0])) rest = rest.slice(1);
    const head = rest[0];
    if (head && PREFIXES.has(head)) { rest = rest.slice(1); continue; }
    if (head === "env" && rest.length > 1) {
      let index = 1;
      while (index < rest.length && (ASSIGNMENT.test(rest[index]) || isFlag(rest[index]))) index++;
      // Bare `env` (or `env -i`) prints the environment; keep it as the program.
      if (index < rest.length) { rest = rest.slice(index); continue; }
    }
    if (head === "timeout" && rest[1]) { rest = rest.slice(2); continue; }
    if (head === "command" && rest[1] && !isFlag(rest[1])) { rest = rest.slice(1); continue; }
    return rest;
  }
}

/** The step a command's first stage names, before its pipe stages add notes. */
function phrase(words: string[]): Phrase | null {
  const [rawBin, ...args] = program(words);
  if (!rawBin) return null;
  const bin = basename(rawBin).toLowerCase();
  switch (bin) {
    case "cat": case "bat": case "less": case "more": case "nl": case "batcat":
      return { ...read("Reading", "Read", "input"), ...files(positionals(args)) };
    case "head": case "tail": {
      const paths = positionals(args, new Set(["-n", "-c", "--lines", "--bytes"]));
      return { ...read("Reading", "Read", "input"), ...files(paths), notes: [headNote(bin, args)] };
    }
    case "sed": {
      // macOS spells an in-place edit with no backup as `-i ''`.
      const free = positionals(args, new Set(["-e", "-f"])).filter(arg => arg !== "");
      const explicit = flagValue(args, ["-e"]);
      const script = explicit ?? free[0];
      const paths = explicit !== undefined ? free : free.slice(1);
      if (args.some(arg => arg === "--in-place" || (/^-[a-zA-Z]*i/.test(arg) && !arg.startsWith("--"))))
        return { ...write("Editing", "Edited", "a file"), ...files(paths) };
      const note = args.includes("-n") ? linesNote(script) : undefined;
      return { ...read("Reading", "Read", "input"), ...files(paths), notes: note ? [note] : [] };
    }
    case "ls": case "dir": case "tree": case "exa": case "eza": {
      const dir = positionals(args, new Set(["-L", "-I", "--ignore"]))[0];
      return read("Listing", "Listed", dir ? basename(dir) : "directory", { glyph: "folder", path: dir });
    }
    case "rg":
      if (args.includes("--files")) {
        const dir = positionals(args, GREP_VALUED)[0];
        return read("Listing", "Listed", dir ? `files in ${basename(dir)}` : "files", { glyph: "folder", path: dir });
      }
      return grepPhrase(args);
    case "grep": case "egrep": case "fgrep": case "ag": case "ack":
      return grepPhrase(args);
    case "find": {
      const name = flagValue(args, ["-name", "-iname", "-path", "-ipath", "-regex"]);
      const root = args.find(arg => !arg.startsWith("-"));
      const effect: CommandEffect = args.some(arg => arg === "-delete" || arg === "-exec" || arg === "-execdir") ? "exec" : "read";
      return { ...search(name ? `“${clip(name, 48)}”` : "files", root), effect };
    }
    case "fd": case "fdfind": case "locate": {
      const pattern = positionals(args, new Set(["-e", "--extension", "-t", "--type", "-d", "--max-depth", "-E", "--exclude"]))[0];
      return search(pattern ? `“${clip(pattern, 48)}”` : "files");
    }
    case "wc": return { ...read("Counting", "Counted", "input"), ...files(positionals(args)) };
    case "stat": case "file": return { ...read("Inspecting", "Inspected", "a file"), ...files(positionals(args)) };
    case "which": case "whereis": case "type": return read("Locating", "Located", positionals(args)[0], { glyph: "search", verb: "search" });
    case "command": return read("Locating", "Located", positionals(args)[0], { glyph: "search", verb: "search" });
    case "git": return gitPhrase(args);
    case "gh": return ghPhrase(args);
    case "df": return look("Checking", "Checked", "disk space", { path: positionals(args)[0] });
    case "du": { const target = positionals(args, new Set(["-d", "--max-depth"]))[0]; return look("Measuring", "Measured", target ? `size of ${basename(target)}` : "disk usage", { path: target }); }
    case "pwd": return look("Checking", "Checked", "the working directory");
    case "echo": case "printf": return look("Printing", "Printed", positionals(args).length ? `“${clip(positionals(args).join(" "), 40)}”` : undefined);
    case "env": case "printenv": return look("Checking", "Checked", "the environment");
    case "date": case "uname": case "whoami": case "hostname": case "id": case "sw_vers": case "arch": case "uptime": case "nproc":
      return look("Checking", "Checked", bin);
    case "ps": case "lsof": case "top": case "pgrep": return look("Checking", "Checked", "running processes");
    case "sleep": return look("Waiting", "Waited", args[0] ? `${args[0]}${/^\d+$/.test(args[0]) ? "s" : ""}` : undefined);
    case "jq": case "yq": { const free = positionals(args, new Set(["--arg", "--argjson", "-f"])); return { ...read("Reading", "Read", "JSON"), ...files(free.slice(1)), notes: free[0] ? [`jq ${clip(free[0], 32)}`] : [] }; }
    case "test": case "[": case "[[": return look("Checking", "Checked", "a condition");
    case "export": return look("Setting", "Set", positionals(args).map(arg => arg.split("=")[0]).join(", ") || "variables");
    case "mkdir": return write("Creating", "Created", positionals(args)[0] ? `directory ${basename(positionals(args)[0])}` : "a directory", { path: positionals(args)[0] });
    case "touch": return { ...write("Creating", "Created", undefined), ...files(positionals(args)) };
    case "rm": case "rmdir": case "unlink": case "trash": return { ...write("Deleting", "Deleted", undefined), ...files(positionals(args)) };
    case "mv": case "cp": case "ln": case "rsync": case "ditto": {
      const free = positionals(args);
      const verbs: Record<string, [string, string]> = { mv: ["Moving", "Moved"], cp: ["Copying", "Copied"], ln: ["Linking", "Linked"], rsync: ["Syncing", "Synced"], ditto: ["Copying", "Copied"] };
      const [doing, done] = verbs[bin];
      return write(doing, done, free.length >= 2 ? `${basename(free[0])} to ${basename(free[free.length - 1])}` : free[0] ? basename(free[0]) : "files");
    }
    case "chmod": case "chown": return write("Changing", "Changed", `permissions on ${positionals(args).slice(1).map(basename).join(", ") || "files"}`);
    case "tee": return { ...write("Writing", "Wrote", undefined), ...files(positionals(args)) };
    case "curl": case "wget": case "http": case "xh": {
      const url = positionals(args, new Set(["-X", "-H", "-d", "--data", "-o", "--output", "-u", "-A", "--header", "-w", "-F", "--form"]))[0];
      let target = url;
      try { if (url) { const parsed = new URL(url); target = `${parsed.host}${parsed.pathname === "/" ? "" : parsed.pathname}`; } } catch { /* not a URL */ }
      return exec("Fetching", "Fetched", target ? clip(target, 56) : undefined, { glyph: "globe" });
    }
    case "open": return exec("Opening", "Opened", positionals(args)[0] ? basename(positionals(args)[0]) : undefined);
    case "kill": case "pkill": case "killall": return exec("Stopping", "Stopped", positionals(args)[0] ?? "a process");
    case "source": case ".": return exec("Loading", "Loaded", positionals(args)[0] ? basename(positionals(args)[0]) : undefined);
    default: return runPhrase(rawBin, args);
  }
}

/** What a pipe stage does to the stream it reads, in words. */
function stageNote(words: string[]): { note?: string; effect: CommandEffect } {
  const [rawBin, ...args] = program(words);
  const bin = basename(rawBin ?? "").toLowerCase();
  switch (bin) {
    case "head": case "tail": return { note: headNote(bin, args), effect: "read" };
    case "grep": case "egrep": case "fgrep": case "rg": case "ag": {
      const explicit = flagValue(args, ["-e", "--regexp"]);
      const pattern = explicit ?? positionals(args, GREP_VALUED)[0];
      const invert = args.some(arg => /^-[a-zA-Z]*v/.test(arg) && !arg.startsWith("--")) || args.includes("--invert-match");
      return { note: pattern ? `${invert ? "without" : "filtered"}: ${clip(pattern, 28)}` : undefined, effect: "read" };
    }
    case "wc": return { note: args.includes("-l") ? "counted lines" : "counted", effect: "read" };
    case "jq": case "yq": return { note: "parsed JSON", effect: "read" };
    case "sort": case "uniq": case "cut": case "tr": case "awk": case "sed": case "column": case "cat": case "less": case "more": case "nl": case "fmt": case "fold": case "rev": case "base64": case "xxd": case "od": case "strings":
      return { effect: bin === "sed" && args.some(arg => /^-[a-zA-Z]*i/.test(arg)) ? "write" : "read" };
    case "tee": { const path = positionals(args)[0]; return { note: path ? `writes ${basename(path)}` : undefined, effect: path ? "write" : "read" }; }
    case "xargs": {
      // The command xargs runs once per line decides what the stage does.
      const valued = new Set(["-n", "-I", "-P", "-L", "-d", "-E", "-s"]);
      let start = 0;
      while (start < args.length && isFlag(args[start])) start += valued.has(args[start]) ? 2 : 1;
      const inner = phrase(args.slice(start));
      return { note: inner ? `for each line: ${inner.doing.toLowerCase()} ${inner.target ?? ""}`.trim() : "piped to xargs", effect: inner?.effect ?? "exec" };
    }
    default: return { note: rawBin ? `piped to ${basename(rawBin)}` : undefined, effect: "exec" };
  }
}

function stronger(a: CommandEffect, b: CommandEffect): CommandEffect {
  return EFFECT_RANK[b] > EFFECT_RANK[a] ? b : a;
}

/** Redirections in words, and whether one of them writes a file. */
function redirectNotes(redirects: Redirect[]): { notes: string[]; writes: boolean } {
  const notes: string[] = [];
  let writes = false;
  for (const { op, target } of redirects) {
    if (!op.includes(">") || !target) continue;
    const fd = /^(\d+|&)?/.exec(op)?.[1];
    if (target === "/dev/null") {
      const note = fd === "2" ? "errors hidden" : "output hidden";
      if (!notes.includes(note)) notes.push(note);
      continue;
    }
    if (/^\/dev\/(stdout|stderr|tty)$/.test(target)) continue;
    writes = true;
    notes.push(`writes ${basename(target)}`);
  }
  return { notes, writes };
}

const CACHE = new Map<string, CommandReading>();
const CACHE_LIMIT = 500;

/** Read a command into its steps. Cached: the same string is the same reading. */
export function readCommand(command: string): CommandReading {
  const cached = CACHE.get(command);
  if (cached) return cached;
  const reading = readUncached(command);
  CACHE.set(command, reading);
  if (CACHE.size > CACHE_LIMIT) CACHE.delete(CACHE.keys().next().value!);
  return reading;
}

function readUncached(command: string): CommandReading {
  const script = unwrapShell(command.trim());
  const pieces = splitShell(script);
  const steps: CommandStep[] = [];
  let directory: string | undefined;
  let previousOp: Operator | null = null;
  for (let index = 0; index < pieces.length;) {
    // One pipeline: this piece and every piece joined to it by `|`.
    const stages = [pieces[index]];
    while (stages[stages.length - 1].op === "|" && index + stages.length < pieces.length) stages.push(pieces[index + stages.length]);
    index += stages.length;
    const end = stages[stages.length - 1].op;
    const opened = previousOp;
    previousOp = end;
    const first = lex(stages[0].text);
    const words = program(first.words);
    const bin = basename(words[0] ?? "").toLowerCase();
    // Whatever runs inside `$(…)`, backticks or a heredoc runs too, wherever
    // it is written: a recognised read around it does not make it a read.
    const nested = runsNestedCode(stages.flatMap(stage => [stage.text, ...(stage.bodies ?? [])]));
    const loaded = first.words.some(word => RUNNING_ENV.test(word));
    if (!nested && (bin === "cd" || bin === "pushd") && stages.length === 1) {
      directory = words[1] ? shortPath(words[1]) : "~";
      continue;
    }
    if (!nested && bin === "popd" && stages.length === 1) { directory = undefined; continue; }
    if (!nested && (bin === "set" || bin === "exit" || bin === "wait" || (bin === "" && !first.redirects.length))) continue;
    if (!nested && (bin === "true" || bin === ":") && stages.length === 1) {
      if (opened === "||" && steps.length) steps[steps.length - 1].notes.push("failure ignored");
      continue;
    }
    const { notes: phraseNotes = [], ...named } = phrase(first.words) ?? runPhrase(words[0] ?? stages[0].text, []);
    const notes: string[] = [];
    if (directory) notes.push(`in ${directory}`);
    notes.push(...phraseNotes);
    let effect = named.effect;
    if (nested) {
      effect = "exec";
      notes.push("runs a nested command");
    }
    if (loaded) effect = "exec";
    const own = redirectNotes(first.redirects);
    notes.push(...own.notes);
    if (own.writes) effect = stronger(effect, "write");
    for (const stage of stages.slice(1)) {
      const lexed = lex(stage.text);
      const piped = stageNote(lexed.words);
      if (piped.note) notes.push(piped.note);
      effect = stronger(effect, piped.effect);
      const redirected = redirectNotes(lexed.redirects);
      notes.push(...redirected.notes);
      if (redirected.writes) effect = stronger(effect, "write");
    }
    if (end === "&") notes.push("in the background");
    // A read that writes a file is no longer a read the reader can skip.
    const verb = effect === "read" ? named.verb : "run";
    steps.push({ ...named, verb, effect, notes, text: stages.map(stage => stage.text).join(" | ") });
  }
  const effect = steps.reduce<CommandEffect>((strongest, step) => stronger(strongest, step.effect), "read");
  return { script, steps, effect };
}

/**
 * The step a chain is named by: the first one with the strongest effect.
 * `cd app && bun run build` is a build, `git status && rm -rf x` a deletion.
 */
export function leadStep(reading: CommandReading): CommandStep | undefined {
  return reading.steps.find(step => step.effect === reading.effect) ?? reading.steps[0];
}

/** The row-level shape of a whole command: its lead step, and how many more. */
export function commandPhrase(reading: CommandReading): Omit<CommandStep, "notes" | "text"> | undefined {
  const lead = leadStep(reading);
  if (!lead) return undefined;
  const more = reading.steps.length - 1;
  const readOnly = reading.effect === "read";
  const verb: ToolVerb = readOnly ? reading.steps[0].verb : "run";
  return {
    verb,
    glyph: lead.glyph,
    doing: lead.doing,
    done: lead.done,
    target: more > 0 ? `${lead.target ?? ""} + ${more} more step${more === 1 ? "" : "s"}`.trim() : lead.target,
    path: more > 0 ? undefined : lead.path,
    effect: reading.effect,
  };
}
