import { describe, expect, it } from "vitest";
import { classifyErrorKind, describeError, providerFromText, usageResetHint } from "./errors";
import type { UsageSnapshot } from "./usage";

const snapshot = (usedPercent: number, resetsInSeconds?: number, label = "Weekly"): UsageSnapshot => ({
  windows: [{ id: label.toLowerCase(), label, usedPercent, resetsInSeconds }],
});

describe("classifyErrorKind", () => {
  it("detects usage/quota exhaustion in real-world phrasings", () => {
    for (const text of [
      "Error: rate limit exceeded",
      "You have hit your weekly usage limit",
      "429 Too Many Requests",
      "insufficient_quota: you exceeded your current quota",
      "Usage limit reached for this plan",
      "You're out of credits",
    ]) {
      expect(classifyErrorKind(text)).toBe("usage-limit");
    }
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
});

describe("usageResetHint", () => {
  it("reports the most-constrained window with a known reset", () => {
    const snap: UsageSnapshot = {
      windows: [
        { id: "5h", label: "5h", usedPercent: 40, resetsInSeconds: 600 },
        { id: "weekly", label: "Weekly", usedPercent: 100, resetsInSeconds: 90000 },
      ],
    };
    expect(usageResetHint(snap)).toBe("The Weekly window resets in 1d 1h.");
  });

  it("returns undefined without windows or resets", () => {
    expect(usageResetHint(null)).toBeUndefined();
    expect(usageResetHint({ windows: [] })).toBeUndefined();
    expect(usageResetHint(snapshot(100))).toBeUndefined();
  });
});

describe("describeError", () => {
  it("names the provider and reset time for usage limits", () => {
    const result = describeError("rate limit exceeded", { provider: "Claude", snapshot: snapshot(100, 7200) });
    expect(result.kind).toBe("usage-limit");
    expect(result.title).toBe("Claude usage limit reached");
    expect(result.message).toContain("your Claude plan's");
    expect(result.message).toContain("resets in 2h");
    expect(result.message).toContain("Switch to another model or provider");
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
