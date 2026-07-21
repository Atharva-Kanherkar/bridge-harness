import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UsageWidget } from "./UsageWidget";
import type { UsageHistoryEntry, UsageSnapshot } from "../usage";

describe("UsageWidget", () => {
  it("keeps both providers visible and states unknown limits without fabrication", () => {
    const html = renderToStaticMarkup(<UsageWidget usage={{}} />);
    expect(html).toContain("Codex");
    expect(html).toContain("Claude");
    expect(html).toContain("Limit unknown");
    expect(html).not.toContain("0% used");
    expect(html).toContain("pointer-events-none");
    expect(html).not.toContain("group-hover:");
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
});
