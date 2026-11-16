// The chat usage dot reads the same provider overviews as the menu bar: the
// ring headlines the tightest live window, stale and errored reads never turn
// into a percentage, and the card draws one marked bar per quota window.
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ProviderUsageOverviews, UsageOverviewSnapshot, UsageQuotaWindow } from "../protocol/generated/protocol";
import { UsageDot, liveUsedPercent, orderedSnapshots, snapshotKey, usageTier, worstLiveUsage } from "./UsageDot";

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

describe("extra accounts", () => {
  const work = (windows: UsageQuotaWindow[], overrides: Partial<UsageOverviewSnapshot> = {}) =>
    snapshot("claude", windows, { account: "work@example.com", plan: "Max", accountId: "work", accountLabel: "Work", ...overrides });

  it("lists every signed-in account under its agent, own login first", () => {
    const own = snapshot("claude", [window("weekly", "Weekly", 12)]);
    const personal = snapshot("codex", [window("weekly", "Weekly", 30)], { account: "me@example.com", accountId: "personal", accountLabel: "Personal" });
    const codex = snapshot("codex", [window("weekly", "Weekly", 5)]);
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(work([window("session", "5-hour", 81)]), personal, own, codex)} nowMs={NOW_MS} />);
    const order = ["Codex usage", "Codex · Personal usage", "Claude usage", "Claude · Work usage"].map(label => html.indexOf(`aria-label="${label}"`));
    expect(order.every(index => index >= 0), html).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(html).toContain("work@example.com · Max");
    expect(html).toContain("81% used");
    // The tightest window may belong to an account; the ring names it.
    expect(html).toContain("81% of Claude · Work 5-hour used");
  });

  it("never shows the own login's Sign in or reset offer on an account row", () => {
    const adapters = [{ id: "claude", label: "Claude", available: true, authState: "signed_out", version: "mock", capabilities: [], unavailableReason: null, models: [] }] as never;
    const html = renderToStaticMarkup(<UsageDot overviews={overviews(work([], { observedAt: null, account: null, error: "Not read yet. Refresh to load this account's limits." }))} adapters={adapters} onRefresh={() => {}} nowMs={NOW_MS} />);
    expect(html).toContain("Claude · Work usage");
    expect(html).not.toContain(">Sign in<");
    expect(html).toContain("Refresh to load this account");
  });

  /** Fuzz the card's ordering against arbitrary backend lists: rows are
   *  never dropped or duplicated, each agent's own login leads its group,
   *  and agents keep the menu bar's order. */
  it("orders any mix of rows without losing or repeating one", () => {
    let seed = 0x2f6e2b1;
    const rand = (n: number) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
    const providers = ["codex", "claude", "cursor", "opencode"];
    for (let run = 0; run < 2_000; run += 1) {
      const rows = Array.from({ length: rand(9) }, () => {
        const account = rand(3) === 0 ? null : ["work", "personal", "x-1"][rand(3)];
        return snapshot(providers[rand(4)], [], account ? { accountId: account, accountLabel: account.toUpperCase() } : {});
      });
      const out = orderedSnapshots(overviews(...rows));
      const keys = out.map(snapshotKey);
      expect(new Set(keys).size).toBe(keys.length);
      expect(new Set(keys)).toEqual(new Set(rows.map(snapshotKey)));
      const providerIndex = out.map(row => providers.indexOf(row.provider));
      expect([...providerIndex].sort((a, b) => a - b)).toEqual(providerIndex);
      for (const provider of providers) {
        const group = out.filter(row => row.provider === provider);
        const firstAccount = group.findIndex(row => !!row.accountId);
        if (firstAccount >= 0) expect(group.slice(firstAccount).every(row => !!row.accountId)).toBe(true);
      }
    }
  });
});
