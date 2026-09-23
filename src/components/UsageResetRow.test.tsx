import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { UsageOverviewSnapshot } from "../protocol/generated/protocol";
import { UsageResetRow, resetSummary } from "./UsageResetRow";

const empty = { tokens: { status: "unavailable" as const }, costMicrousd: { status: "unavailable" as const }, models: [] };
const snapshot = (resetCredits?: UsageOverviewSnapshot["resetCredits"]): UsageOverviewSnapshot => ({
  schemaVersion: 1, generatedAt: Math.floor(Date.now() / 1000), provider: "codex", account: "a@example.test", observedAt: Math.floor(Date.now() / 1000),
  coverage: "test", windows: [], today: empty, month: empty, error: null, resetCredits,
});

describe("UsageResetRow", () => {
  it("keeps missing and zero quiet while count-only remains actionable", () => {
    expect(resetSummary(snapshot())).toBeNull();
    expect(resetSummary(snapshot({ availableCount: 0, detailsKnown: true, credits: [], nextExpiresAt: null }))).toBeNull();
    const countOnly = snapshot({ availableCount: 2, detailsKnown: false, credits: [], nextExpiresAt: null });
    expect(resetSummary(countOnly)).toBe("2 resets banked");
    expect(renderToStaticMarkup(<UsageResetRow snapshot={countOnly} />)).not.toContain("disabled=\"\"");
  });

  it("shows expiry and explains when Claude's grant needs a limit", () => {
    const claude = { ...snapshot({ availableCount: 1, detailsKnown: true, nextExpiresAt: 1_800_086_400, credits: [{
      id: "grant_1", title: "Banked reset", expiresAt: 1_800_086_400, grantedAt: null,
      clears: ["five_hour"], usableNow: false, requiresLimit: true, program: "cedar_ember",
    }] }), provider: "claude" };
    const html = renderToStaticMarkup(<UsageResetRow snapshot={claude} />);
    expect(html).toContain("earliest expires");
    expect(html).toContain("Available when you reach a limit");
    expect(html).toContain("disabled=\"\"");
  });
});
