/**
 * What a command's output is, when it is something better drawn than dumped.
 *
 * `gh pr view 70 --json title,body,files` answers with a JSON document whose
 * body is one long escaped string: `\n` literals, a markdown table flattened
 * into pipes. The terminal block shows that faithfully and unreadably. This
 * module recognises the shapes worth drawing (a PR, an issue, a workflow run,
 * a list of them, or any JSON at all) and reads them into plain data for the
 * renderer.
 *
 * It runs when a reader opens a call, never at ingestion: parsing output on
 * every streamed delta would be work nobody looks at. React-free, so it tests
 * as data.
 */

import { ghSubcommand } from "./command";

export type StructuredKind = "gh-pr" | "gh-issue" | "gh-run" | "gh-list" | "json";

export interface StructuredOutput {
  kind: StructuredKind;
  data: unknown;
  /** The document was cut off (`| head -c 20000`) and closed by `repairTruncatedJson`. */
  truncated: boolean;
}

/** Past this, the raw view is the honest one: a tree of half a megabyte helps nobody. */
const MAX_BYTES = 512 * 1024;

type Bag = Record<string, unknown>;

const isBag = (value: unknown): value is Bag => !!value && typeof value === "object" && !Array.isArray(value);
const str = (value: unknown): string | undefined => (typeof value === "string" && value.trim() ? value : undefined);
const num = (value: unknown): number | undefined => (typeof value === "number" && Number.isFinite(value) ? value : undefined);

/**
 * Close a JSON document that was cut off mid-stream: finish the open string,
 * drop a dangling key, comma or partial literal, and close every open bracket.
 * Returns `undefined` when the text was not truncated JSON to begin with (a
 * mismatched bracket is malformed, not cut off).
 */
export function repairTruncatedJson(text: string): string | undefined {
  const stack: string[] = [];
  let inString = false;
  let escaped = false;
  for (let index = 0; index < text.length; index++) {
    const ch = text[index];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "{") stack.push("}");
    else if (ch === "[") stack.push("]");
    else if (ch === "}" || ch === "]") {
      if (stack.pop() !== ch) return undefined;
    }
  }
  if (!stack.length) return undefined;
  let out = text;
  if (inString) {
    if (escaped) out = out.slice(0, -1);
    out = out.replace(/\\u[0-9a-fA-F]{0,3}$/, "");
    out += '"';
  }
  const inObject = stack[stack.length - 1] === "}";
  for (;;) {
    const before = out;
    out = out.replace(/\s+$/, "").replace(/,$/, "");
    // A key whose value never arrived.
    out = out.replace(/([{,])\s*"(?:[^"\\]|\\.)*"\s*:$/, "$1");
    // A key that never got its colon: a string straight after `{` or `,` in an object.
    if (inObject) out = out.replace(/([{,])\s*"(?:[^"\\]|\\.)*"$/, "$1");
    // A literal or number cut short.
    out = out.replace(/([:[,])\s*(?:t|tr|tru|f|fa|fal|fals|n|nu|nul|-|-?\d+\.|-?\d+[eE][+-]?)$/, "$1");
    // A value left dangling after its colon.
    out = out.replace(/([{,])\s*"(?:[^"\\]|\\.)*"\s*:\s*$/, "$1");
    if (out === before) break;
  }
  return out + [...stack].reverse().join("");
}

function listShaped(data: unknown): boolean {
  return Array.isArray(data) && data.length > 0 && data.every(entry => isBag(entry)
    && (num(entry.number) !== undefined || num(entry.databaseId) !== undefined)
    && !!(str(entry.title) ?? str(entry.displayTitle) ?? str(entry.name)));
}

/** Recognise output worth drawing. `null` means: show it as it came. */
export function detectStructuredOutput(command: string | undefined, output: string | undefined): StructuredOutput | null {
  const trimmed = output?.trim();
  if (!trimmed || trimmed.length > MAX_BYTES) return null;
  if (trimmed[0] !== "{" && trimmed[0] !== "[") return null;
  let data: unknown;
  let truncated = false;
  try {
    data = JSON.parse(trimmed);
  } catch {
    const repaired = repairTruncatedJson(trimmed);
    if (repaired === undefined) return null;
    try {
      data = JSON.parse(repaired);
      truncated = true;
    } catch {
      return null;
    }
  }
  if (!data || typeof data !== "object") return null;
  const { noun, verb } = command ? ghSubcommand(command) : {};
  if (verb === "view" && isBag(data)) {
    if (noun === "pr" && (str(data.title) || num(data.number) !== undefined)) return { kind: "gh-pr", data, truncated };
    if (noun === "issue" && (str(data.title) || num(data.number) !== undefined)) return { kind: "gh-issue", data, truncated };
    if (noun === "run") return { kind: "gh-run", data, truncated };
  }
  if (verb === "list" && (noun === "pr" || noun === "issue" || noun === "run") && listShaped(data)) return { kind: "gh-list", data, truncated };
  return { kind: "json", data, truncated };
}

/* ── Reading the shapes ─────────────────────────────────────────────────── */

export interface PrFile {
  path: string;
  additions?: number;
  deletions?: number;
  change?: string;
}

export interface GhSummary {
  repo?: string;
  number?: number;
  title?: string;
  state?: string;
  draft?: boolean;
  author?: string;
  url?: string;
  body?: string;
  head?: string;
  base?: string;
  additions?: number;
  deletions?: number;
  changedFiles?: number;
  commits?: number;
  reviews?: number;
  reviewDecision?: string;
  labels: string[];
  files: PrFile[];
  checks?: { passed: number; failed: number; pending: number };
  comments?: number;
}

function login(value: unknown): string | undefined {
  return isBag(value) ? str(value.login) ?? str(value.name) : str(value);
}

function repoOf(url: string | undefined): string | undefined {
  const match = url ? /github\.com\/([^/]+\/[^/]+)\//.exec(url) : null;
  return match?.[1];
}

function count(value: unknown): number | undefined {
  if (Array.isArray(value)) return value.length;
  if (isBag(value)) return num(value.totalCount) ?? (Array.isArray(value.nodes) ? value.nodes.length : undefined);
  return num(value);
}

const CHANGE: Record<string, string> = { ADDED: "A", MODIFIED: "M", DELETED: "D", RENAMED: "R", COPIED: "C", CHANGED: "M" };

/** A PR or an issue, read into the fields a card shows. */
export function ghSummary(data: unknown): GhSummary {
  const bag = isBag(data) ? data : {};
  const files = (Array.isArray(bag.files) ? bag.files : []).flatMap(entry => {
    if (!isBag(entry)) return [];
    const path = str(entry.path) ?? str(entry.filename);
    return path ? [{ path, additions: num(entry.additions), deletions: num(entry.deletions), change: CHANGE[String(entry.changeType ?? "").toUpperCase()] }] : [];
  });
  const rollup = Array.isArray(bag.statusCheckRollup) ? bag.statusCheckRollup.filter(isBag) : [];
  const checks = rollup.length ? rollup.reduce<{ passed: number; failed: number; pending: number }>((tally, check) => {
    const outcome = String(check.conclusion ?? check.state ?? check.status ?? "").toUpperCase();
    if (["SUCCESS", "NEUTRAL", "SKIPPED"].includes(outcome)) tally.passed += 1;
    else if (["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"].includes(outcome)) tally.failed += 1;
    else tally.pending += 1;
    return tally;
  }, { passed: 0, failed: 0, pending: 0 }) : undefined;
  const sum = (key: "additions" | "deletions") => files.length && files.every(file => file[key] !== undefined) ? files.reduce((total, file) => total + (file[key] ?? 0), 0) : undefined;
  const url = str(bag.url);
  return {
    repo: repoOf(url) ?? (isBag(bag.headRepository) ? str(bag.headRepository.nameWithOwner) : undefined),
    number: num(bag.number),
    title: str(bag.title),
    state: str(bag.state),
    draft: bag.isDraft === true,
    author: login(bag.author),
    url,
    body: typeof bag.body === "string" ? bag.body : undefined,
    head: str(bag.headRefName),
    base: str(bag.baseRefName),
    additions: num(bag.additions) ?? sum("additions"),
    deletions: num(bag.deletions) ?? sum("deletions"),
    changedFiles: num(bag.changedFiles) ?? (files.length || undefined),
    commits: count(bag.commits),
    reviews: count(bag.reviews),
    reviewDecision: str(bag.reviewDecision),
    labels: (Array.isArray(bag.labels) ? bag.labels : isBag(bag.labels) && Array.isArray(bag.labels.nodes) ? bag.labels.nodes : [])
      .flatMap(label => (isBag(label) && str(label.name) ? [str(label.name)!] : typeof label === "string" ? [label] : [])),
    files,
    checks,
    comments: count(bag.comments),
  };
}

export interface RunSummary {
  title?: string;
  workflow?: string;
  status?: string;
  conclusion?: string;
  branch?: string;
  event?: string;
  url?: string;
  jobs: Array<{ name: string; status?: string; conclusion?: string }>;
}

export function runSummary(data: unknown): RunSummary {
  const bag = isBag(data) ? data : {};
  return {
    title: str(bag.displayTitle) ?? str(bag.name),
    workflow: str(bag.workflowName),
    status: str(bag.status),
    conclusion: str(bag.conclusion),
    branch: str(bag.headBranch),
    event: str(bag.event),
    url: str(bag.url),
    jobs: (Array.isArray(bag.jobs) ? bag.jobs : []).flatMap(job => (isBag(job) && str(job.name)
      ? [{ name: str(job.name)!, status: str(job.status), conclusion: str(job.conclusion) }]
      : [])),
  };
}

export interface ListRow {
  id: string;
  title: string;
  state?: string;
}

export function listRows(data: unknown): ListRow[] {
  return (Array.isArray(data) ? data : []).flatMap(entry => {
    if (!isBag(entry)) return [];
    const id = num(entry.number) ?? num(entry.databaseId);
    const title = str(entry.title) ?? str(entry.displayTitle) ?? str(entry.name);
    if (id === undefined || !title) return [];
    return [{ id: num(entry.number) !== undefined ? `#${id}` : String(id), title, state: str(entry.conclusion) ?? str(entry.state) ?? str(entry.status) }];
  });
}
