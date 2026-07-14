import { formatReset, type UsageSnapshot } from "./usage";

// Turn a raw provider/adapter error string into something a human can act on.
// The important case: when a provider (Codex/Claude) is out of subscription
// usage, say so — and, when we already know the rate-limit windows, when it
// resets — instead of surfacing a bare stack-tracey "we errored out".

export type ErrorKind = "usage-limit" | "auth" | "network" | "generic";

export interface DescribedError {
  kind: ErrorKind;
  title: string;
  message: string;
}

// Deliberately specific phrases — a bare "limit" or "rate" would misfire on
// ordinary prose. These match what OpenAI/Anthropic and their CLIs actually say.
const USAGE_LIMIT = /(rate[\s_-]?limit|usage[\s_-]?limit|\bquota\b|out of (?:usage|credits?|tokens)|insufficient[\s_]?quota|too many requests|\b429\b|(?:monthly|weekly|daily|plan|usage) limit|limit reached|reached your .*limit|exceeded your|usage cap|no (?:remaining|credits))/i;
const AUTH = /(unauthorized|\b401\b|\b403\b|not (?:logged|signed) in|authentication failed|invalid api key|expired (?:token|credentials)|please (?:log|sign) ?in|logged out|re-?authenticate)/i;
const NETWORK = /(econnrefused|etimedout|timed out|network error|dns|offline|failed to fetch|connection (?:refused|reset|closed)|unreachable|socket hang up)/i;

export function classifyErrorKind(raw: string | undefined | null): ErrorKind {
  const text = raw ?? "";
  if (USAGE_LIMIT.test(text)) return "usage-limit";
  if (AUTH.test(text)) return "auth";
  if (NETWORK.test(text)) return "network";
  return "generic";
}

/** Best-effort provider name from the error text, when the caller didn't supply one. */
export function providerFromText(raw: string | undefined | null): string | undefined {
  const text = raw ?? "";
  if (/(claude|anthropic)/i.test(text)) return "Claude";
  if (/(codex|openai|\bgpt\b)/i.test(text)) return "Codex";
  return undefined;
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

  if (kind === "usage-limit") {
    const who = provider ? `your ${provider} plan's` : "your plan's";
    const reset = usageResetHint(options.snapshot);
    const message = [
      `You're out of usage on ${who} current limit.`,
      reset,
      "Switch to another model or provider to keep working, or wait for the limit to reset.",
    ]
      .filter(Boolean)
      .join(" ");
    return { kind, title: provider ? `${provider} usage limit reached` : "Usage limit reached", message };
  }

  if (kind === "auth") {
    return {
      kind,
      title: provider ? `Sign in to ${provider}` : "Authentication needed",
      message: `${provider ?? "The provider"} needs you to sign in again. Run its /login command, then retry.`,
    };
  }

  if (kind === "network") {
    return {
      kind,
      title: "Connection problem",
      message: "Couldn't reach the provider. Check your network connection and try again.",
    };
  }

  return { kind: "generic", title: "Something went wrong", message: text || "The adapter reported an error." };
}
