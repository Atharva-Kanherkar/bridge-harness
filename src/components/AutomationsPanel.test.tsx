// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AutomationCatalog } from "../types";
import { AutomationsPanel } from "./AutomationsPanel";

const catalog: AutomationCatalog = {
  automations: [
    {
      id: "task-1", provider: "claude", name: "Summarize overnight CI failures", prompt: "Summarize overnight CI failures.",
      schedule: { kind: "cron", expression: "7 9 * * 1-5", human: "Weekdays at 09:07" }, status: "active", recurring: true,
      createdAt: 1_800_000_000_000, nextRunAt: null, lastRunAt: null, cwds: [], model: null, effort: null, runs: [],
    },
    {
      id: "auto-1", provider: "codex", name: "Nightly dependency audit", prompt: "Audit dependencies.",
      schedule: { kind: "rrule", expression: "FREQ=DAILY;BYHOUR=3;BYMINUTE=15", human: "Daily at 03:15" }, status: "paused", recurring: true,
      createdAt: 1_800_000_100_000, nextRunAt: null, lastRunAt: null, cwds: ["/tmp/repo"], model: "gpt-5.3-codex", effort: "high",
      runs: [{ id: "thread-1", automationId: "auto-1", status: "COMPLETED", title: "Deps clean", summary: "No CVEs", createdAt: 1_800_000_050_000 }],
    },
  ],
  providers: [
    { provider: "claude", available: true, detail: "~/.claude/scheduled_tasks.json", count: 1, capabilities: ["create", "edit", "delete"] },
    { provider: "codex", available: true, detail: "~/.codex/sqlite/codex.db", count: 1, capabilities: ["pause", "resume", "delete"] },
    { provider: "cursor", available: false, detail: "Cursor has no native automations feature", count: 0, capabilities: [] },
    { provider: "opencode", available: false, detail: "OpenCode has no native automations feature", count: 0, capabilities: [] },
  ],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.spyOn(bridgeApi, "automationCatalog").mockResolvedValue(catalog);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("AutomationsPanel", () => {
  it("renders native schedules and explicit provider capabilities", () => {
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={catalog} />);
    expect(html).toContain("Summarize overnight CI failures");
    expect(html).toContain("Nightly dependency audit");
    expect(html).toContain("Cursor has no native automations feature");
    // Bridge ships four harnesses; every one is named, including the two with
    // no automations store of their own.
    expect(html).toContain("OpenCode has no native automations feature");
    expect(html).toContain("No native automation controls");
    expect(html).toContain("New Claude automation");
    expect(html).not.toContain("Run now");
    // Human phrasing and status still carry the row.
    expect(html).toContain("Weekdays at 09:07");
    expect(html).toContain("Daily at 03:15");
    expect(html).toContain("paused");
  });

  it("withholds capability chips and controls from a store it cannot read", () => {
    const unreadable: AutomationCatalog = {
      automations: [],
      providers: catalog.providers.map(state => state.provider === "claude"
        ? { ...state, available: false, detail: "~/.claude/scheduled_tasks.json is unreadable: bad json", count: 0 }
        : state),
    };
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={unreadable} />);
    expect(html).toContain("is unreadable");
    expect(html).not.toContain("New Claude automation");
    expect(html).not.toContain(">Create<");
  });

  it("shows only actions advertised by each owning harness", async () => {
    await act(async () => root.render(<AutomationsPanel initialCatalog={catalog} />));
    const row = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(name))!;
    await act(async () => row("Summarize overnight CI failures").click());
    const claudeCard = row("Summarize overnight CI failures").closest("article")!;
    expect(claudeCard.textContent).toContain("Edit");
    expect(claudeCard.textContent).toContain("Delete");
    expect(claudeCard.textContent).not.toContain("Pause");
    expect(claudeCard.textContent).not.toContain("Run now");

    await act(async () => row("Nightly dependency audit").click());
    const codexCard = row("Nightly dependency audit").closest("article")!;
    expect(codexCard.textContent).toContain("Resume");
    expect(codexCard.textContent).toContain("Delete");
    expect(codexCard.textContent).not.toContain("Edit");
    expect(codexCard.textContent).not.toContain("Run now");
  });

  it("opens the native Claude creation form only because create is advertised", async () => {
    await act(async () => root.render(<AutomationsPanel initialCatalog={catalog} />));
    const create = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes("New Claude automation"))!;
    await act(async () => create.click());
    expect(document.body.textContent).toContain("Saved directly to Claude Code's native schedule file");
    expect(document.body.querySelector('[aria-label="Automation prompt"]')).toBeTruthy();
    expect(document.body.querySelector('[aria-label="Automation cron schedule"]')).toBeTruthy();
  });

  it("renders the empty state when no automations exist", () => {
    const html = renderToStaticMarkup(<AutomationsPanel initialCatalog={{ automations: [], providers: catalog.providers }} />);
    expect(html).toContain("No native automations yet");
  });
});
