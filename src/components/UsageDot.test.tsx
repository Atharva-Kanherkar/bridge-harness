// The chat usage dot reads the same provider overviews as the menu bar: the
// ring headlines the tightest live window, stale and errored reads never turn
// into a percentage, and the card draws one marked bar per quota window.
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ProviderUsageOverviews, UsageOverviewSnapshot, UsageQuotaWindow } from "../protocol/generated/protocol";
import { UsageDot, liveUsedPercent, usageTier, worstLiveUsage } from "./UsageDot";

const NOW = 1_800_000_000;
const NOW_MS = NOW * 1000;
const empty = { tokens: { status: "unavailable" as const }, costMicrousd: { status: "unavailable" as const }, models: [] };

function window(id: string, label: string, value: number | null, overrides: Partial<UsageQuotaWindow> = {}): UsageQuotaWindow {
  return { id, label, usedPercent: value == null ? { status: "unavailable" } : { value, source: "reported", status: "current" }, resetsAt: NOW + 3600, windowMinutes: null, ...overrides };
}

function snapshot(provider: string, windows: UsageQuotaWindow[], overrides: Partial<UsageOverviewSnapshot> = {}): UsageOverviewSnapshot {
  return { schemaVersion: 1, generatedAt: NOW, provider, account: "dev@example.com", plan: "plus", observedAt: NOW - 30, coverage: "test", windows, today: empty, month: empty, error: null, ...overrides };
}

function overviews(...providers: UsageOverviewSnapshot[]): ProviderUsageOverviews {
  return { schemaVersion: 1, generatedAt: NOW, providers };
}

describe("worstLiveUsage", () => {
  it("headlines the tightest live window, not the first slot", () => {
    const codex = snapshot("codex", [window("session", "5-hour", 0), window("weekly", "Weekly", 63)]);
    const worst = worstLiveUsage(overviews(codex), NOW);
    expect(worst?.window.id).toBe("weekly");
    expect(worst?.used).toBe(63);
  });

  it("spans providers and keeps a reported zero as a real zero", () => {
    const codex = snapshot("codex", [window("session", "5-hour", 0)]);
    const cursor = snapshot("cursor", [window("total", "Total", 68.5, { resetsAt: null })]);
    expect(worstLiveUsage(overviews(codex, cursor), NOW)?.provider).toBe("cursor");
    expect(worstLiveUsage(overviews(codex), NOW)?.used).toBe(0);
  });

  it("ignores stale, expired, unavailable, old and errored readings instead of inventing a percentage", () => {
    const stale = snapshot("codex", [window("weekly", "Weekly", 90, { usedPercent: { value: 90, source: "reported", status: "stale" } })]);
    const expired = snapshot("codex", [window("weekly", "Weekly", 90, { resetsAt: NOW - 1 })]);
    const unavailable = snapshot("codex", [window("weekly", "Weekly", null)]);
    const old = snapshot("codex", [window("weekly", "Weekly", 90)], { observedAt: NOW - 601 });
    const failed = snapshot("claude", [window("weekly", "Weekly", 90)], { error: "Reconnect the provider." });
    for (const value of [stale, expired, unavailable, old, failed]) expect(worstLiveUsage(overviews(value), NOW)).toBeNull();
    expect(worstLiveUsage(null, NOW)).toBeNull();
    expect(liveUsedPercent(window("weekly", "Weekly", 140), NOW)).toBe(100);
  });

  it("tiers at 70 and 90", () => {
    expect(usageTier(null)).toBe("unknown");
    expect(usageTier(69)).toBe("ok");
    expect(usageTier(70)).toBe("warning");
    expect(usageTier(90)).toBe("critical");
  });
});

describe("UsageDot", () => {
  it("names its state without colour and stays quiet before any reading arrives", () => {
    const html = renderToStaticMarkup(<UsageDot overviews={null} nowMs={NOW_MS} />);
    expect(html).toContain("Open usage — no live usage yet");
    expect(html).toContain("Loading usage");
    expect(html).not.toContain("0%");
    expect(html).toContain("text-muted-foreground");
    expect(html).toContain("u-glass-popover");
    expect(html).toContain("pointer-events-none");
  });

  it("colours the ring by the worst window and says which one it is", () => {
    const codex = snapshot("codex", [window("session", "5-hour", 0), window("weekly", "Weekly", 63)]);
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(codex)} nowMs={NOW_MS} />);
    expect(html).toContain("Open usage — healthy, 63% of Codex Weekly used");
    expect(html).toContain("text-success");
    expect(html).toContain("63% · Codex Weekly");
    const critical = snapshot("claude", [window("weekly", "Weekly", 94)]);
    expect(renderToStaticMarkup(<UsageDot overviews={overviews(critical)} nowMs={NOW_MS} />)).toContain("critical, 94% of Claude Weekly used");
  });

  it("draws one marked bar per window with used, left and reset, in the menu bar's order", () => {
    const codex = snapshot("codex", [window("session", "5-hour", 0), window("weekly", "Weekly", 63, { resetsAt: NOW + 2 * 86400 + 3 * 3600 })]);
    const cursor = snapshot("cursor", [window("total", "Total", 68.5, { resetsAt: null })], { plan: "free" });
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(cursor, codex)} nowMs={NOW_MS} />);
    expect(html.indexOf("Codex usage")).toBeLessThan(html.indexOf("Cursor usage"));
    expect(html).toContain("0% used");
    expect(html).toContain("100% left");
    expect(html).toContain("63% used");
    expect(html).toContain("37% left");
    expect(html).toContain("resets in 2d 3h");
    expect(html).toContain("69% used");
    expect(html).toContain("dev@example.com · plus");
    expect(html).toContain("dev@example.com · free");
    expect(html).toContain("bg-chart-codex");
    expect(html).toContain("bg-chart-cursor");
    expect(html.match(/data-quota-marker="50"/g)).toHaveLength(3);
    expect(html.match(/data-quota-marker="75"/g)).toHaveLength(3);
    // A 0% window draws its track and markers but no fill.
    expect(html).not.toContain("width:0%");
  });

  it("keeps stale readings as history and states a failed provider's error", () => {
    const codex = snapshot("codex", [window("weekly", "Weekly", 42, { usedPercent: { value: 42, source: "reported", status: "stale" } })]);
    const claude = snapshot("claude", [], { observedAt: null, account: null, plan: null, error: "Provider session is unavailable in Keychain. Reconnect the provider." });
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(codex, claude)} nowMs={NOW_MS} />);
    expect(html).toContain("Open usage — no live usage yet");
    expect(html).toContain("42% used");
    expect(html).toContain("Stale");
    expect(html).not.toContain("% left");
    expect(html).toContain("Reconnect the provider.");
  });

  it("offers Sign in for a signed-out adapter and Open Usage when routed", () => {
    const codex = snapshot("codex", [], { error: "Sign in to Codex." });
    const adapters = [{ id: "codex", label: "Codex", available: false, authState: "signed_out", version: "mock", capabilities: [], unavailableReason: null, models: [] }] as never;
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(codex)} adapters={adapters} onOpenUsage={() => {}} onRefresh={() => {}} nowMs={NOW_MS} />);
    expect(html).toContain("Sign in");
    expect(html).toContain("Open Usage");
    expect(html).toContain("Refresh usage");
  });
});
