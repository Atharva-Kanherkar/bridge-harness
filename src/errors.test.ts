import { describe, expect, it } from "vitest";
import { authModeFromText, classifyErrorKind, describeError, errorMessage, providerFromText, upstreamFromText, usageHeadroomHint, usageResetHint } from "./errors";
import type { UsageSnapshot } from "./usage";

const snapshot = (usedPercent: number, resetsInSeconds?: number, label = "Weekly"): UsageSnapshot => ({
  windows: [{ id: label.toLowerCase(), label, usedPercent, resetsInSeconds, source: "reported" }],
  source: "reported",
  capturedAt: "2026-07-16T10:00:00Z",
});

describe("classifyErrorKind", () => {
  it("detects usage/quota exhaustion in real-world phrasings", () => {
    for (const text of [
      "You have hit your weekly usage limit",
      "insufficient_quota: you exceeded your current quota",
      "Usage limit reached for this plan",
      "You're out of credits",
      "Your credit balance is too low to run this request",
    ]) {
      expect(classifyErrorKind(text)).toBe("usage-limit");
    }
  });

  // A throttle and a spent plan are different facts with different repairs,
  // and one regex used to answer for both — so a 429 that a retry would have
  // cleared told the user their subscription was gone.
  it("keeps throttling separate from exhaustion", () => {
    for (const text of [
      "Error: rate limit exceeded",
      "Rate limit reached for requests per minute",
      "You exceeded your rate limit",
      "429 Too Many Requests",
      "Request failed: 429 (retry-after: 30)",
      "You are sending requests too quickly: 60 requests per minute",
    ]) {
      expect(classifyErrorKind(text)).toBe("rate-limit");
    }
  });

  it("reads a frame that says both as exhaustion", () => {
    expect(classifyErrorKind("429: monthly quota exceeded for this organization")).toBe("usage-limit");
  });

  it("detects auth and network problems", () => {
    expect(classifyErrorKind("401 Unauthorized")).toBe("auth");
    expect(classifyErrorKind("Please log in to continue")).toBe("auth");
    expect(classifyErrorKind("connection refused")).toBe("network");
    expect(classifyErrorKind("failed to fetch")).toBe("network");
  });

  it("does not misfire on ordinary prose", () => {
    expect(classifyErrorKind("Could not parse the file")).toBe("generic");
    expect(classifyErrorKind("The rate of change is high")).toBe("generic");
    expect(classifyErrorKind("")).toBe("generic");
    expect(classifyErrorKind(undefined)).toBe("generic");
  });
});

describe("providerFromText", () => {
  it("sniffs the provider when present", () => {
    expect(providerFromText("anthropic: overloaded")).toBe("Claude");
    expect(providerFromText("openai rate limit")).toBe("Codex");
    expect(providerFromText("something generic")).toBeUndefined();
  });

  it("prefers the runtime over the model vendor it relays", () => {
    expect(providerFromText("opencode: openai returned 429")).toBe("OpenCode");
  });
});

describe("upstreamFromText", () => {
  it("names the model vendor a bring-your-own-key runtime was calling", () => {
    expect(upstreamFromText("opencode: openai returned 429")).toBe("OpenAI");
    expect(upstreamFromText("anthropic overloaded_error")).toBe("Anthropic");
    expect(upstreamFromText("the tool call failed")).toBeUndefined();
  });
});

describe("authModeFromText", () => {
  it("tells an API key apart from a subscription sign-in", () => {
    expect(authModeFromText("401 invalid api key provided")).toBe("api-key");
    expect(authModeFromText("OPENAI_API_KEY is not set")).toBe("api-key");
    expect(authModeFromText("You are not logged in. Run /login.")).toBe("subscription");
    expect(authModeFromText("session has expired, please sign in")).toBe("subscription");
    expect(authModeFromText("403 Forbidden")).toBe("unknown");
  });
});

describe("usageResetHint", () => {
  it("reports the most-constrained window with a known reset", () => {
    const snap: UsageSnapshot = {
      windows: [
        { id: "5h", label: "5h", usedPercent: 40, resetsInSeconds: 600, source: "reported" },
        { id: "weekly", label: "Weekly", usedPercent: 100, resetsInSeconds: 90000, source: "reported" },
      ],
      source: "reported",
      capturedAt: "2026-07-16T10:00:00Z",
    };
    expect(usageResetHint(snap)).toBe("The Weekly window resets in 1d 1h.");
  });

  it("returns undefined without windows or resets", () => {
    expect(usageResetHint(null)).toBeUndefined();
    expect(usageResetHint({ windows: [], source: "reported", capturedAt: "2026-07-16T10:00:00Z" })).toBeUndefined();
    expect(usageResetHint(snapshot(100))).toBeUndefined();
  });
});

describe("errorMessage", () => {
  it("unwraps the daemon-host error envelope object to its message", () => {
    expect(
      errorMessage({ code: 1001, kind: "git", message: "Git: fatal: not a git repository" }),
    ).toBe("Git: fatal: not a git repository");
  });

  it("unwraps the same envelope when it arrives as a JSON string", () => {
    const envelope =
      '{"code":1001,"kind":"git","message":"Git: fatal: not a git repository (or any of the parent directories): .git"}';
    expect(errorMessage(envelope)).toBe(
      "Git: fatal: not a git repository (or any of the parent directories): .git",
    );
  });

  it("leaves plain strings, Errors, and non-envelope values alone", () => {
    expect(errorMessage("plain failure")).toBe("plain failure");
    expect(errorMessage(new Error("thrown"))).toBe("thrown");
    expect(errorMessage(42)).toBe("42");
  });

  it("falls back to the raw text when the envelope carries no usable message", () => {
    expect(errorMessage('{"code":1001,"kind":"git"}')).toBe('{"code":1001,"kind":"git"}');
    expect(errorMessage({ code: 1001 })).toBe("[object Object]");
    expect(errorMessage('{"message":"   "}')).toBe('{"message":"   "}');
    expect(errorMessage("{not json")).toBe("{not json");
  });
});

describe("describeError", () => {
  it("names the provider and reset time for usage limits", () => {
    const result = describeError("You've hit your usage limit.", { provider: "Claude", snapshot: snapshot(100, 7200) });
    expect(result.kind).toBe("usage-limit");
    expect(result.title).toBe("Claude usage limit reached");
    expect(result.message).toContain("Claude reports that this account has reached its usage limit");
    expect(result.message).toContain("resets in 2h");
    expect(result.message).toContain("Switch to another model or provider");
  });

  // The report that started this: a 429 arrived, Bridge said the plan was
  // spent, and the user went looking for a subscription to top up.
  it("does not call a rate limit an exhausted plan", () => {
    const result = describeError("429 Too Many Requests", { provider: "Codex", snapshot: snapshot(12, 7200) });
    expect(result.kind).toBe("rate-limit");
    expect(result.title).toBe("Codex is rate limiting");
    expect(result.message).toContain("throttling requests");
    expect(result.message).toContain("not evidence that the plan's usage is spent");
    expect(result.message).toContain("Weekly window is 12% used");
    expect(result.message).not.toMatch(/out of usage|reached its usage limit/);
  });

  it("claims no headroom it cannot see", () => {
    const bare = describeError("429 Too Many Requests", { provider: "Codex" });
    expect(bare.message).not.toContain("usage left");
    const full = describeError("429 Too Many Requests", { provider: "Codex", snapshot: snapshot(100, 600) });
    expect(full.message).not.toContain("usage left");
  });

  // Switching a chat to OpenCode does not hand OpenCode the bill for an
  // OpenAI account it was merely calling on the user's key.
  it("attributes an upstream vendor's limit upstream", () => {
    const result = describeError("opencode: openai returned 429 usage limit reached", { provider: "OpenCode" });
    expect(result.kind).toBe("usage-limit");
    expect(result.message).toContain("the upstream OpenAI account it calls");
    expect(result.message).not.toContain("OpenCode plan");
  });

  it("reads a Codex OpenAI limit as the account Codex signs into", () => {
    const result = describeError("openai: you exceeded your current quota", { provider: "Codex" });
    expect(result.message).toContain("this account");
    expect(result.message).not.toContain("upstream");
  });

  it("prescribes /login only for a subscription sign-in", () => {
    const subscription = describeError("You are not logged in. Run /login.", { provider: "Codex" });
    expect(subscription.authMode).toBe("subscription");
    expect(subscription.title).toBe("Sign in to Codex");
    expect(subscription.message).toContain("/login");

    const key = describeError("401 invalid api key provided", { provider: "Codex" });
    expect(key.kind).toBe("auth");
    expect(key.authMode).toBe("api-key");
    expect(key.title).toBe("Codex rejected its API key");
    expect(key.message).toContain("rejected the API key");
    expect(key.message).toContain("a different credential");

    const unclear = describeError("403 Forbidden", { provider: "OpenCode" });
    expect(unclear.authMode).toBe("unknown");
    expect(unclear.message).toContain("if it uses an API key");
    expect(unclear.message).toContain("sign in again");
  });

  it("falls back to sniffed provider and stays graceful without a snapshot", () => {
    const result = describeError("openai: you exceeded your current quota");
    expect(result.kind).toBe("usage-limit");
    expect(result.title).toBe("Codex usage limit reached");
    expect(result.message).not.toContain("undefined");
  });

  it("passes generic errors through as their raw text", () => {
    expect(describeError("boom: file not found")).toEqual({
      kind: "generic",
      title: "Something went wrong",
      message: "boom: file not found",
    });
  });
});
