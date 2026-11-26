import { formatReset, type UsageSnapshot } from "./usage";

// Turn a raw provider/adapter error string into something a human can act on.
// The important case: when a provider (Codex/Claude) is out of subscription
// usage, say so — and, when we already know the rate-limit windows, when it
// resets — instead of surfacing a bare stack-tracey "we errored out".

export type ErrorKind = "usage-limit" | "rate-limit" | "auth" | "network" | "generic";

/**
 * Which credential the provider rejected, when its own words say.
 *
 * A subscription sign-in and an API key are different credentials with
 * different repairs, and `/login` fixes exactly one of them. Guessing costs
 * the reader a round trip through the wrong one — which is what happened to a
 * user who had just replaced a Codex subscription with an API key in a
 * terminal and was told, repeatedly, to sign in again.
 */
export type AuthMode = "api-key" | "subscription" | "unknown";

export interface DescribedError {
  kind: ErrorKind;
  title: string;
  message: string;
  /** Present on `auth`: which credential the text blames. */
  authMode?: AuthMode;
}

// Deliberately specific phrases — a bare "limit" or "rate" would misfire on
// ordinary prose. These match what OpenAI/Anthropic and their CLIs actually say.
//
// Exhaustion and throttling are two different facts and used to share one
// pattern, so a 429 — which every provider returns for bursts a retry clears —
// produced "you're out of usage on your plan". Exhaustion is checked first:
// a frame that says both ("429: monthly quota exceeded") is exhaustion.
const USAGE_LIMIT = /(usage[\s_-]?limit|\bquota\b|out of (?:usage|credits?|tokens)|insufficient[\s_]?quota|(?:monthly|weekly|daily|plan|usage) limit|usage cap|no (?:remaining|credits)|credit balance is too low)/i;
const RATE_LIMIT = /(rate[\s_-]?limit|too many requests|\b429\b|retry[\s_-]?after|requests per (?:minute|second|hour))/i;
const AUTH = /(unauthorized|\b401\b|\b403\b|forbidden|not (?:logged|signed) in|authentication failed|invalid api key|api[\s_-]?key|expired (?:token|credentials)|please (?:log|sign) ?in|logged out|re-?authenticate)/i;
const NETWORK = /(econnrefused|etimedout|timed out|network error|dns|offline|failed to fetch|connection (?:refused|reset|closed)|unreachable|socket hang up)/i;

// What the text blames, when it names a credential at all.
const API_KEY_AUTH = /(api[\s_-]?key|apikey|_API_KEY|bearer token)/i;
const SUBSCRIPTION_AUTH = /(not (?:logged|signed) in|please (?:log|sign) ?in|logged out|sign in again|subscription|oauth|session (?:has )?expired|re-?authenticate|\/login)/i;

/** The human-readable text of anything thrown across the Tauri boundary. */
export function errorMessage(value: unknown): string {
  const message = envelopeMessage(value);
  if (message) return message;
  return value instanceof Error ? value.message : String(value);
}

// Daemon-host mode wraps every command error in a JSON envelope —
// {"code":1001,"kind":"git","message":"…"} (daemon_host.rs host_error_envelope) —
// which reached cards and toasts verbatim. The `message` field is the human
// text; the envelope around it is not.
function envelopeMessage(value: unknown): string | undefined {
  const candidate = typeof value === "string" ? parseEnvelope(value) : value;
  if (
    candidate !== null &&
    typeof candidate === "object" &&
    !Array.isArray(candidate) &&
    typeof (candidate as { message?: unknown }).message === "string"
  ) {
    const message = (candidate as { message: string }).message.trim();
    if (message) return message;
  }
  return undefined;
}

function parseEnvelope(raw: string): unknown {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return undefined;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return undefined;
  }
}

export function classifyErrorKind(raw: string | undefined | null): ErrorKind {
  const text = raw ?? "";
  if (USAGE_LIMIT.test(text)) return "usage-limit";
  if (RATE_LIMIT.test(text)) return "rate-limit";
  if (AUTH.test(text)) return "auth";
  if (NETWORK.test(text)) return "network";
  return "generic";
}

/** Whether a kind is a wait rather than a fault — the two that draw in warning tone. */
export function isThrottleKind(kind: ErrorKind): boolean {
  return kind === "usage-limit" || kind === "rate-limit";
}

/** Which credential an auth failure blames, from the provider's own words. */
export function authModeFromText(raw: string | undefined | null): AuthMode {
  const text = raw ?? "";
  // A key named explicitly wins: "invalid api key" is unambiguous, while
  // "unauthorized" beside it is just the HTTP status carrying it.
  if (API_KEY_AUTH.test(text)) return "api-key";
  if (SUBSCRIPTION_AUTH.test(text)) return "subscription";
  return "unknown";
}

/** Best-effort provider name from the error text, when the caller didn't supply one. */
export function providerFromText(raw: string | undefined | null): string | undefined {
  const text = raw ?? "";
  // Runtimes first: an OpenCode frame relaying an OpenAI 429 names both, and
  // the runtime is the one the reader switched to.
  // Cursor is deliberately absent: "cursor" is an ordinary word in editor
  // errors, and a sniff is only worth having when it cannot be wrong for free.
  if (/opencode/i.test(text)) return "OpenCode";
  if (/(claude|anthropic)/i.test(text)) return "Claude";
  if (/(codex|openai|\bgpt\b)/i.test(text)) return "Codex";
  return undefined;
}

/** The model vendor a runtime label speaks to on its own account, if it has one. */
function vendorOf(provider: string | undefined): string | undefined {
  if (!provider) return undefined;
  if (/codex/i.test(provider)) return "OpenAI";
  if (/claude/i.test(provider)) return "Anthropic";
  // OpenCode and Cursor front whatever account the user configured; they have
  // no vendor of their own, so an upstream name in the text is never redundant.
  return undefined;
}

/**
 * A model vendor named inside the text — which, under a bring-your-own-key
 * runtime, is whose quota actually ran out.
 *
 * OpenCode relaying `openai: 429` is not evidence about an OpenCode plan, and
 * saying so sent a user looking for a subscription they never bought.
 */
export function upstreamFromText(raw: string | undefined | null): string | undefined {
  const text = raw ?? "";
  if (/(openai|\bgpt-|o[34]-mini)/i.test(text)) return "OpenAI";
  if (/anthropic/i.test(text)) return "Anthropic";
  if (/(google|gemini|vertex)/i.test(text)) return "Google";
  return undefined;
}

/** Whose account the limit belongs to, said only as far as the evidence goes. */
function accountPhrase(provider: string | undefined, text: string): string {
  const upstream = upstreamFromText(text);
  if (upstream && upstream !== vendorOf(provider)) return `the upstream ${upstream} account it calls`;
  return "this account";
}

/** "The Weekly window resets in 2d 3h." for the most-constrained window we know about. */
export function usageResetHint(snapshot: UsageSnapshot | null | undefined): string | undefined {
  if (!snapshot || !snapshot.windows.length) return undefined;
  const byPressure = [...snapshot.windows].sort((a, b) => b.usedPercent - a.usedPercent);
  for (const window of byPressure) {
    const reset = formatReset(window.resetsInSeconds);
    if (reset) return `The ${window.label} window ${reset}.`;
  }
  return undefined;
}

/**
 * The evidence that a throttle is only a throttle: the account's own meter,
 * cited when it shows room left. Silent when the meter is missing or full,
 * because "you still have usage" is a claim and this is the only thing that
 * backs it.
 */
export function usageHeadroomHint(snapshot: UsageSnapshot | null | undefined): string | undefined {
  if (!snapshot || !snapshot.windows.length) return undefined;
  const pressed = [...snapshot.windows].sort((a, b) => b.usedPercent - a.usedPercent)[0];
  if (pressed.usedPercent >= 95) return undefined;
  return `Its ${pressed.label} window is ${Math.round(pressed.usedPercent)}% used, so there is usage left.`;
}

export interface DescribeOptions {
  /** Human provider label, e.g. "Claude". Falls back to sniffing the text. */
  provider?: string;
  /** Latest rate-limit snapshot for that provider, for a reset countdown. */
  snapshot?: UsageSnapshot | null;
}

export function describeError(raw: string | undefined | null, options: DescribeOptions = {}): DescribedError {
  const text = (raw ?? "").trim();
  const kind = classifyErrorKind(text);
  const provider = options.provider?.trim() || providerFromText(text);

  const source = provider ?? "The provider";

  if (kind === "usage-limit") {
    const message = [
      `${source} reports that ${accountPhrase(provider, text)} has reached its usage limit.`,
      usageResetHint(options.snapshot),
      "Switch to another model or provider to keep working, or wait for the limit to reset.",
    ]
      .filter(Boolean)
      .join(" ");
    return { kind, title: provider ? `${provider} usage limit reached` : "Usage limit reached", message };
  }

  if (kind === "rate-limit") {
    // A throttle says the requests were too close together. It says nothing
    // about the plan behind them, and the reader is about to decide whether to
    // go buy more usage — so the difference is stated, not implied.
    const message = [
      `${source} is throttling requests for ${accountPhrase(provider, text)}: too many in a short window.`,
      "That is a rate limit, not evidence that the plan's usage is spent.",
      usageHeadroomHint(options.snapshot),
      "Wait a moment and retry, or switch to another model or provider.",
    ]
      .filter(Boolean)
      .join(" ");
    return { kind, title: provider ? `${provider} is rate limiting` : "Rate limited", message };
  }

  if (kind === "auth") {
    const authMode = authModeFromText(text);
    if (authMode === "api-key") {
      return {
        kind,
        authMode,
        title: provider ? `${provider} rejected its API key` : "API key rejected",
        message: `${source} rejected the API key it is configured with. Check that the key is valid and can reach this model, then retry. A subscription sign-in uses a different credential and does not repair this key.`,
      };
    }
    if (authMode === "subscription") {
      return {
        kind,
        authMode,
        title: provider ? `Sign in to ${provider}` : "Authentication needed",
        message: `${source} needs you to sign in again. Run its /login command, then retry.`,
      };
    }
    return {
      kind,
      authMode,
      title: provider ? `${provider} rejected the credentials` : "Authentication needed",
      message: `${source} rejected the credentials for this session without saying which. If it signs in with a subscription, sign in again; if it uses an API key, check the key is still set and valid. Then retry.`,
    };
  }

  if (kind === "network") {
    return {
      kind,
      title: "Connection problem",
      message: "Couldn't reach the provider. Check your network connection and try again.",
    };
  }

  return { kind: "generic", title: "Something went wrong", message: friendlyError(text) };
}

// --- what a person reads ------------------------------------------------------

/** What a person reads when nothing better can be said. */
export const GENERIC_FAILURE = "Something went wrong. Try again.";

/** Longest message shown as is; anything longer is cut to its first sentence. */
const MAX_FRIENDLY_CHARS = 160;

// Faults with a known, plain explanation. Checked before the text is shown at
// all, because their raw form is a code (`EACCES`, `SQLITE_BUSY`) nobody reads.
const KNOWN_FAULTS: Array<[RegExp, string]> = [
  [/(permission denied|\beacces\b|\beperm\b|operation not permitted)/i, "Bridge doesn't have permission to do that. Check the folder's permissions and try again."],
  [/(no space left|\benospc\b|disk (?:is )?full)/i, "The disk is full. Free up some space and try again."],
  [/(database is locked|sqlite_busy|database table is locked)/i, "Bridge is busy saving something else. Try again in a moment."],
  [/(no such file or directory|\benoent\b|os error 2\b)/i, "A file or folder Bridge needed is missing."],
  [/(panicked at|called `(?:result|option)::unwrap|stack backtrace|traceback \(most recent call last\))/i, GENERIC_FAILURE],
];

// Category prefixes the runtime puts in front of the real sentence
// (`BridgeError`'s Display, Git's own `fatal:`, a JavaScript error's name).
const NOISE_PREFIX = /^(?:(?:uncaught\s+)?(?:\w*error|error|fatal|warning|git|gh|database|i\/o|io|adapter|pty|credential policy|invalid|exception|rpc)\s*(?:\[[^\]]*\])?\s*:\s*)+/i;

// What marks text as written for a developer: code, structures, paths with
// line numbers, hex, identifiers in SCREAMING_CASE, call syntax.
const TECHNICAL = /([{}[\]<>]|::|=>|```|\$\{|\b0x[0-9a-f]+\b|\bat\s+\S+:\d+|\.(?:rs|ts|tsx|js|mjs|py|go):\d+|\b\w+\(\)|\b(?:undefined|null|NaN|serde|deserializ\w*|unwrap|errno|stderr|stdout|exit (?:code|status)|status code)\b|\bline \d+ column \d+\b)/i;

// SCREAMING_CASE identifiers (`SQLITE_BUSY`, `ERR_INVALID_ARG`), checked case
// by case so a file named `scheduled_tasks.json` is still a word.
const IDENTIFIER = /\b[A-Z][A-Z0-9]*_[A-Z0-9_]+\b/;

/** An absolute path as its last part, so a message names the file, not the disk. */
function shortenPaths(text: string): string {
  return text.replace(/(?:~|\/(?:Users|home|private|var|tmp|opt|Applications|Library|Volumes))(?:\/[^\s'"`:,;()]+)+/g, (path) => {
    const parts = path.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? path;
  });
}

function sentenceCase(text: string): string {
  const trimmed = text.trim().replace(/[\s:;,-]+$/, "");
  if (!trimmed) return trimmed;
  // "src/App.tsx is not a file" keeps its file name as written.
  const first = trimmed.split(/\s/, 1)[0];
  // So do "macOS", "gh" and the other names that are lowercase on purpose.
  const keep = /[/._]\w/.test(first) || /^[a-z]+[A-Z]/.test(first) || /^(?:gh|git|npm|bun|bunx|node|cargo|bridged|pnpm|yarn)$/.test(first);
  const capital = keep ? trimmed : trimmed[0].toUpperCase() + trimmed.slice(1);
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`;
}

/** The first sentence, when the whole is too long to read at a glance. */
function firstSentence(text: string): string {
  const match = text.match(/^.+?[.!?](?=\s|$)/);
  return match ? match[0] : text;
}

/**
 * The one short, plain sentence a person reads when something failed.
 *
 * Raw error text — a Rust error's Display, a JSON envelope, a provider's HTTP
 * body, a parser's position report — reached toasts, cards and banners
 * verbatim, and read like a crash log. This keeps what a person can act on and
 * drops the rest: known faults get a plain explanation, prefixes and paths
 * come off, and anything that still reads like code becomes `fallback`.
 *
 * `fallback` should say what failed ("Couldn't save the note."), so the reader
 * still learns something when the detail is unusable.
 */
export function friendlyError(value: unknown, fallback: string = GENERIC_FAILURE): string {
  const raw = errorMessage(value ?? "").trim();
  if (!raw) return fallback;

  // Bridge's own daemon, before the network check: "unreachable" is both.
  if (/(bridge daemon (?:connection was lost|is unreachable)|broken pipe|daemon (?:is )?not running)/i.test(raw)) {
    return "Bridge's background service isn't responding. Try again in a moment.";
  }
  for (const [pattern, message] of KNOWN_FAULTS) {
    if (pattern.test(raw)) return message === GENERIC_FAILURE ? fallback : message;
  }
  const kind = classifyErrorKind(raw);
  // GitHub is not a model provider: its sign-in and its throttle have their own repairs.
  if (/(github|\bgh\b)/i.test(raw)) {
    if (kind === "auth") return "GitHub needs you to sign in again. Run gh auth login in a terminal, then try again.";
    if (kind === "rate-limit" || kind === "usage-limit") return "GitHub is rate limiting requests. Try again in a few minutes.";
  }
  const plain = plainSentence(raw);
  if (kind === "generic") return plain ?? fallback;
  // A sentence the backend already wrote for people ("OpenCode Go rejected the
  // saved sign-in. Run opencode auth login…") says more than a canned one; a
  // status code or an error name does not.
  if (plain && !CODED.test(raw)) return plain;
  if (kind === "network") return "Couldn't connect. Check your network connection and try again.";
  return describeError(raw).message;
}

// What marks text as a machine's report rather than a sentence for people:
// an HTTP status, an errno name, an OS error number, a URL.
const CODED = /(\b[1-5]\d{2}\b|\bE[A-Z]{3,}\b|\berrno\b|\bos error\b|https?:\/\/)/;

/** The raw text as one plain sentence, or undefined when it reads like code. */
function plainSentence(raw: string): string | undefined {
  const line = raw.split(/\r?\n/).map((part) => part.trim()).find(Boolean) ?? "";
  let text = shortenPaths(line)
    .replace(NOISE_PREFIX, "")
    .replace(/\s*\((?:os error|code|errno)[^)]*\)/gi, "")
    .trim();
  // "Could not open the repository: failed to resolve path: …" — the outer
  // clause says what failed in the reader's terms; the inner ones say how.
  // Whole sentences before the colon are kept as they are.
  const colon = text.search(/:\s/);
  if (colon > 0) {
    const head = text.slice(0, colon);
    const stop = Math.max(head.lastIndexOf(". "), head.lastIndexOf("! "), head.lastIndexOf("? "));
    if (stop > 0) text = head.slice(0, stop + 1);
    else if (head.split(/\s+/).length >= 3) text = head;
  }
  if (text.length > MAX_FRIENDLY_CHARS) text = firstSentence(text);
  // One bare word ("refused", "offline") tells a person nothing they can act on.
  if (text.split(/\s+/).length < 2) return undefined;
  if (!text || text.length > MAX_FRIENDLY_CHARS || TECHNICAL.test(text) || IDENTIFIER.test(text) || !/[a-z]{3}/i.test(text)) return undefined;
  return sentenceCase(text);
}
