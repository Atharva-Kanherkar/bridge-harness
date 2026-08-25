import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UsageWidget } from "./UsageWidget";
import type { CacheDiagnostic, UsageHistoryEntry, UsageSnapshot } from "../usage";

describe("UsageWidget", () => {
  it("keeps both providers visible and states unknown limits without fabrication", () => {
    const html = renderToStaticMarkup(<UsageWidget usage={{}} />);
    expect(html).toContain("Codex");
    expect(html).toContain("Claude");
    expect(html).toContain("Limit unknown");
    expect(html).not.toContain("0% used");
    expect(html).toContain("pointer-events-none");
    expect(html).not.toContain("group-hover:");
    expect(html).toContain("rounded-window-control");
    expect(html).toContain("Close usage health details");
    expect(html).toContain("Hide usage widget");
  });

  it("labels reported meters and explains context pressure", () => {
    const snapshot: UsageSnapshot = {
      windows: [{ id: "weekly", label: "Weekly", usedPercent: 82, resetsLabel: "resets Friday", source: "reported" }],
      planType: "pro",
      model: "gpt-5",
      source: "reported",
      capturedAt: "2026-07-16T10:00:00Z",
    };
    const html = renderToStaticMarkup(<UsageWidget usage={{ codex: snapshot }} contextPercent={76} contextSource="measured" />);
    expect(html).toContain("82% used");
    expect(html).toContain("Reported");
    expect(html).toContain("High pressure");
    expect(html).toContain("At least 75%");
    expect(html).toContain("Measured");
  });

  it("renders work-unit traceability and estimated projections", () => {
    const history: UsageHistoryEntry[] = [{ id: 1, workUnit: "turn-51", harness: "codex", model: "gpt-5", outcome: "completed", source: "reported", totalTokens: 150, contextPercent: 45, createdAt: "2026-07-16T10:00:00Z" }];
    const snapshot: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 80, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:10:00Z" };
    const html = renderToStaticMarkup(<UsageWidget usage={{ codex: snapshot }} history={history} samples={{ codex: [
      { usedPercent: 70, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 75, capturedAt: "2026-07-16T10:05:00Z" },
      { usedPercent: 80, capturedAt: "2026-07-16T10:10:00Z" },
    ] }} />);
    expect(html).toContain("turn-51");
    expect(html).toContain("gpt-5");
    expect(html).toContain("completed");
    expect(html).toContain("Estimated from 3 samples");
  });

  it("renders cache ratios, prefix provenance, and unknown provider cost without fake savings", () => {
    const cache: CacheDiagnostic = {
      key: "codex-cache", harness: "codex", model: "gpt-5", role: "worker:implementation",
      taskFamily: "implementation", restorationMode: "fresh", stablePrefixId: "bridge-prompt-v1-deadbeef",
      stablePrefixHash: "deadbeef", promptSchemaVersion: 1, prefixTokenEstimate: 100,
      cacheReadTokens: 120, cacheWriteTokens: 20, uncachedInputTokens: 160,
      cacheHitRatio: 0.4, writeAmortization: 6, observations: 2,
      crossHarnessReuse: ["same_harness"], costSources: [], costCoverage: "unknown",
    };
    const html = renderToStaticMarkup(<UsageWidget usage={{}} cacheDiagnostics={[cache]} />);
    expect(html).toContain("Prompt cache");
    expect(html).toContain("Hit 40%");
    expect(html).toContain("write amortization 6.0×");
    expect(html).toContain("bridge-prompt-v1-deadbeef");
    expect(html).toContain("schema v1");
    expect(html).toContain("Role: Worker · implementation");
    expect(html).toContain("Restore: Fresh");
    expect(html).toContain("Reuse: Same harness");
    expect(html).toContain("Cost unknown — provider did not report it");
    expect(html.toLowerCase()).not.toContain("savings");
  });

  it("discloses when additional prompt groups are hidden", () => {
    const cache = (index: number): CacheDiagnostic => ({
      key: `cache-${index}`, harness: "codex", model: `gpt-${index}`, role: "worker:implementation",
      taskFamily: "implementation", restorationMode: "checkpoint_restored",
      cacheReadTokens: 1, cacheWriteTokens: 0, uncachedInputTokens: 1,
      observations: 1, crossHarnessReuse: [], costSources: [], costCoverage: "unknown",
    });
    const html = renderToStaticMarkup(<UsageWidget usage={{}} cacheDiagnostics={Array.from({ length: 7 }, (_, index) => cache(index))} />);
    expect(html).toContain("Showing 6 of 7 recent prompt groups.");
    expect(html).toContain("Restore: Checkpoint restored");
  });

  it("offers the breakdown entry only with a focused session, and keeps the panel unmounted by default", () => {
    const withSession = renderToStaticMarkup(<UsageWidget usage={{}} contextPercent={76} contextSource="measured" focusedSessionId="session-a" />);
    expect(withSession).toContain("Open context breakdown");
    expect(withSession).not.toContain("reconciling");
    const noSession = renderToStaticMarkup(<UsageWidget usage={{}} contextPercent={76} contextSource="measured" />);
    expect(noSession).not.toContain("Open context breakdown");
  });
});
