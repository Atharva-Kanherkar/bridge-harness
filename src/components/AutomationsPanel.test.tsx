import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { AutomationCatalog } from "../types";
import { AutomationsPanel } from "./AutomationsPanel";

const catalog: AutomationCatalog = {
  automations: [
    {
      id: "task-1", provider: "claude", name: "Summarize overnight CI failures", prompt: "Summarize overnight CI failures.",
      schedule: { kind: "cron", expression: "7 9 * * 1-5", human: "Weekdays at 09:07" }, status: "active", recurring: true,
      createdAt: 1_800_000_000_000, nextRunAt: null, lastRunAt: null, cwds: [], model: null, effort: null, canPause: false, runs: [],
    },
    {
      id: "auto-1", provider: "codex", name: "Nightly dependency audit", prompt: "Audit dependencies.",
      schedule: { kind: "rrule", expression: "FREQ=DAILY;BYHOUR=3;BYMINUTE=15", human: "Daily at 03:15" }, status: "paused", recurring: true,
      createdAt: 1_800_000_100_000, nextRunAt: null, lastRunAt: null, cwds: ["/tmp/repo"], model: "gpt-5.3-codex", effort: "high", canPause: true,
      runs: [{ id: "thread-1", automationId: "auto-1", status: "COMPLETED", title: "Deps clean", summary: "No CVEs", createdAt: 1_800_000_050_000 }],
    },
  ],
  providers: [
    { provider: "claude", available: true, detail: "~/.claude/scheduled_tasks.json", count: 1 },
    { provider: "codex", available: true, detail: "~/.codex/sqlite/codex.db", count: 1 },
    { provider: "opencode", available: false, detail: "OpenCode has no automations feature", count: 0 },
  ],
};

describe("AutomationsPanel", () => {
  it("renders both providers' automations with status and human schedules", () => {
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={catalog} />);
    expect(html).toContain("Summarize overnight CI failures");
    expect(html).toContain("Weekdays at 09:07");
    expect(html).toContain("Nightly dependency audit");
    expect(html).toContain("Daily at 03:15");
    expect(html).toContain("paused");
    expect(html).toContain('aria-label="Filter automations by provider"');
    expect(html).toContain('aria-label="Filter automations by status"');
    expect(html).toContain("All statuses");
  });

  it("shows the provider strip with availability, including unsupported OpenCode", () => {
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={catalog} />);
    expect(html).toContain("Claude Code · 1");
    expect(html).toContain("Codex · 1");
    expect(html).toContain("OpenCode · unavailable");
  });

  it("renders the empty state when no automations exist", () => {
    const html = renderToStaticMarkup(
      <AutomationsPanel initialCatalog={{ automations: [], providers: catalog.providers }} />,
    );
    expect(html).toContain("No automations yet");
  });

  it("offers a quiet catalog link when a handler is provided", () => {
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={catalog} onBrowseCatalog={() => {}} />);
    expect(html).toContain("Browse catalog");
    expect(html).not.toContain('aria-label="Filter resources"');
  });
});
