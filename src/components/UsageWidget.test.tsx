import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UsageWidget } from "./UsageWidget";
import type { CacheDiagnostic, UsageHistoryEntry, UsageSnapshot } from "../usage";
import type { AdapterDescriptor, AuthState } from "../types";

function adapterFixture(id: string, overrides: Partial<AdapterDescriptor> = {}): AdapterDescriptor {
  return { id, label: id, available: true, authState: "signed_in" as AuthState, version: "mock", capabilities: [], unavailableReason: null, models: [], ...overrides };
}

describe("UsageWidget", () => {
  it("keeps both providers visible and states unknown limits without fabrication", () => {
    const html = renderToStaticMarkup(<UsageWidget usage={{}} />);
    expect(html).toContain("Codex");
    expect(html).toContain("Claude");
    expect(html).toContain("Cursor");
    expect(html).toContain("OpenCode");
    expect(html).toContain("Limit unknown");
    expect(html).not.toContain("0% used");
    expect(html).toContain("pointer-events-none");
    expect(html).not.toContain("group-hover:");
    expect(html).toContain("Open usage health details");
    expect(html).toContain("Close usage health details");
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

  it("labels a signed-out provider 'Not signed in' and never claims its limit is merely unknown", () => {
    const adapters: AdapterDescriptor[] = [
      adapterFixture("codex", { authState: "signed_out" }),
      adapterFixture("claude"),
      adapterFixture("cursor"),
      adapterFixture("opencode"),
    ];
    const snapshot: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 40, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:00:00Z" };
    const html = renderToStaticMarkup(<UsageWidget usage={{ claude: snapshot, cursor: snapshot, opencode: snapshot }} adapters={adapters} />);
    expect(html).toContain("Not signed in");
    expect(html).not.toContain("Limit unknown");
  });

  it("still renders 'Limit unknown' for a signed-in provider that has no snapshot yet", () => {
    const adapters: AdapterDescriptor[] = [
      adapterFixture("codex"),
      adapterFixture("claude"),
      adapterFixture("opencode"),
    ];
    const html = renderToStaticMarkup(<UsageWidget usage={{}} adapters={adapters} />);
    expect(html).toContain("Limit unknown");
    expect(html).not.toContain("Not signed in");
  });

  it("renders a not-installed provider distinctly from a signed-out one", () => {
    const adapters: AdapterDescriptor[] = [
      adapterFixture("codex", { available: false, authState: "unknown", unavailableReason: "codex CLI not found on PATH" }),
      adapterFixture("claude", { authState: "signed_out" }),
      adapterFixture("opencode"),
    ];
    const html = renderToStaticMarkup(<UsageWidget usage={{}} adapters={adapters} />);
    expect(html).toContain("Not installed");
    expect(html).toContain("codex CLI not found on PATH");
    expect(html).toContain("Not signed in");
  });

  it("offers sign-in for a signed-out cursor provider instead of calling it absent", () => {
    // Cursor probes by opening a session, so a signed-out install reports
    // available:false and signed_out together — the only shape the real
    // descriptor produces, and the one that used to render as not installed.
    const adapters: AdapterDescriptor[] = [
      adapterFixture("codex"),
      adapterFixture("claude"),
      adapterFixture("cursor", { available: false, authState: "signed_out", unavailableReason: "Cursor 2026.08.25 is installed but not signed in; run cursor-agent login" }),
      adapterFixture("opencode"),
    ];
    const snapshot: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 40, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:00:00Z" };
    const html = renderToStaticMarkup(<UsageWidget usage={{ codex: snapshot, claude: snapshot, opencode: snapshot }} adapters={adapters} />);
    expect(html).toContain(">Sign in</button>");
    expect(html).toContain("Not signed in");
    expect(html).not.toContain("not installed");
    expect(html).not.toContain("Add it in Settings");
    expect(html).not.toContain("Limit unknown");
  });

  it("renders no progress arc for not-installed or signed-out providers", () => {
    const adapters: AdapterDescriptor[] = [
      adapterFixture("codex", { available: false, authState: "unknown" }),
      adapterFixture("claude", { authState: "signed_out" }),
      adapterFixture("opencode", { authState: "signed_out" }),
    ];
    const html = renderToStaticMarkup(<UsageWidget usage={{}} adapters={adapters} />);
    expect(html).not.toContain("stroke-dashoffset");
  });

  it("colors the compact indicator by the worst reported usage, not an average, and names that state in its accessible label", () => {
    const low: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 20, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:00:00Z" };
    const critical: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 95, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:00:00Z" };
    const html = renderToStaticMarkup(<UsageWidget usage={{ codex: low, claude: critical }} />);
    expect(html).toContain("text-destructive");
    expect(html).toContain("Usage health — critical, 95% used");
    // The tier and percent must be in the accessible name itself, not only the
    // hover title — a screen reader never reads `title`.
    expect(html).toContain('aria-label="Open usage health details — critical, 95% used"');
  });

  it("leaves the compact indicator neutral when no provider has reported usage", () => {
    const html = renderToStaticMarkup(<UsageWidget usage={{}} />);
    expect(html).toContain("Usage health — no reported usage yet");
    expect(html).toContain('aria-label="Open usage health details — no reported usage yet"');
    expect(html).not.toContain("text-destructive");
    expect(html).not.toContain("text-warning");
  });
});
