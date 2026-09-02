import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UsageWidget } from "./UsageWidget";
import type { UsageHistoryEntry, UsageSnapshot } from "../usage";
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
    expect(html).toContain("Show more");
    expect(html).not.toContain("Resize usage panel");
  });

  it("renders a flat popup that scrolls its own content instead of clipping it", () => {
    const html = renderToStaticMarkup(<UsageWidget usage={{}} />);
    // Flat surface: the popup carries no shadow-bearing surface class, only a
    // border on the popover token.
    expect(html).not.toContain("u-overlay");
    expect(html).not.toContain("u-glass");
    expect(html).not.toContain("shadow");
    expect(html).toContain("bg-popover");
    // Sized by its content, capped at the viewport, scrolling inside.
    expect(html).toContain("max-h-[80dvh]");
    expect(html).toContain("min-h-0 flex-1 overflow-y-auto");
    // No drag-to-resize affordance survives.
    expect(html).not.toContain("Resize usage panel");
    expect(html).not.toContain('role="separator"');
    expect(html).not.toContain("cursor-ns-resize");
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

  it("states estimated projections up front and keeps work-unit history behind Show more", () => {
    const history: UsageHistoryEntry[] = [{ id: 1, workUnit: "turn-51", harness: "codex", model: "gpt-5", outcome: "completed", source: "reported", totalTokens: 150, contextPercent: 45, createdAt: "2026-07-16T10:00:00Z" }];
    const snapshot: UsageSnapshot = { windows: [{ id: "weekly", label: "Weekly", usedPercent: 80, source: "reported" }], source: "reported", capturedAt: "2026-07-16T10:10:00Z" };
    const html = renderToStaticMarkup(<UsageWidget usage={{ codex: snapshot }} history={history} samples={{ codex: [
      { usedPercent: 70, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 75, capturedAt: "2026-07-16T10:05:00Z" },
      { usedPercent: 80, capturedAt: "2026-07-16T10:10:00Z" },
    ] }} />);
    expect(html).toContain("Estimated from 3 samples");
    // The disclosure body is mounted by Show more, so nothing of it is in the
    // collapsed markup. Its contents are covered in UsageWidget.interaction.
    expect(html).toContain("Show more");
    expect(html).not.toContain("turn-51");
    expect(html).not.toContain("Recent work units");
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

  it("renders a compact ring trigger that still carries the full usage panel", () => {
    const html = renderToStaticMarkup(<UsageWidget compact usage={{}} contextPercent={76} contextSource="measured" />);
    expect(html).not.toContain("w-[390px]");
    expect(html).toContain("rounded-full");
    expect(html).toContain("Open usage health details");
    expect(html).toContain("bottom-full");
    expect(html).toContain("Show more");
    expect(html).not.toContain("Resize usage panel");
    expect(html).toContain("Codex");
    expect(html).toContain("Claude");
    expect(html).toContain("Cursor");
    expect(html).toContain("OpenCode");
    expect(html).toContain("Limit unknown");
    // Cache and history are the disclosure body: mounted only once expanded.
    expect(html).not.toContain("Prompt cache");
    expect(html).not.toContain("Recent work units");
  });
});
