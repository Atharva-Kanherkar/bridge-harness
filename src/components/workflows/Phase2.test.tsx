// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { Workflow, WorkflowRun, WorkflowSummary, WorkflowTemplate } from "../../protocol/generated/protocol";
import { WorkflowsScreen, connectorTrouble, queuePositions } from "../WorkflowsScreen";
import { WorkflowEditor, composeEventTrigger, previewSentence } from "./WorkflowEditor";
import { WorkflowRunView, queuedLine } from "./WorkflowRunView";
import { templateBlocker } from "./WorkflowTemplates";
import { costLine, formatCost, ordinal, presetFor, triggerSummary } from "./format";
import * as externalLinks from "../../externalLinks";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const settle = (ms = 0) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim().includes(label));
const field = <T extends HTMLElement>(label: string) => document.querySelector<T>(`[aria-label="${label}"]`)!;
function change(element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}

describe("the preview sentence", () => {
  it("counts events, not runs, when they are collected", () => {
    const preview = { supported: true, reason: null, windowDays: 7, matches: 30, samples: [], note: null };
    expect(previewSentence(preview)).toBe("Would have run 30 times in the last 7 days.");
    expect(previewSentence(preview, true)).toBe("30 matching events in the last 7 days, collected into one run per scheduled time.");
    expect(previewSentence({ ...preview, note: "At least 30: …" }, true)).toBe("At least 30 matching events in the last 7 days, collected into one run per scheduled time.");
    expect(previewSentence({ ...preview, matches: 1 }, true)).toBe("1 matching event in the last 7 days, collected into one run per scheduled time.");
    expect(previewSentence({ ...preview, matches: 0 }, true)).toBe("No matching events in the last 7 days.");
  });
});

describe("format", () => {
  it("says an event trigger as a person would", () => {
    expect(triggerSummary({ kind: "event", event: "slack_mention" })).toBe("When I'm mentioned in Slack");
    expect(triggerSummary({ kind: "event", event: "slack_thread_reply", channel: "eng", from: "Priya" })).toBe("When someone replies in my Slack thread in #eng from Priya");
    expect(triggerSummary({ kind: "event", event: "github_pull_request_opened", repo: "acme/app" })).toBe("When a pull request is opened in acme/app");
    expect(triggerSummary({ kind: "event", event: "slack_mention", batch: { cron: "0 9 * * 1-5", timezone: "UTC" } })).toBe("When I'm mentioned in Slack, collected and run on weekdays at 09:00");
    expect(presetFor({ kind: "event", event: "slack_mention", batch: { cron: "30 8 * * *", timezone: "UTC" } })).toMatchObject({ preset: "event", time: "08:30" });
  });

  it("shows costs as estimates people can read", () => {
    expect(formatCost(null)).toBeNull();
    expect(formatCost(0)).toBe("$0");
    expect(formatCost(4_000)).toBe("under $0.01");
    expect(formatCost(40_000)).toBe("$0.040");
    expect(formatCost(1_234_000)).toBe("$1.23");
    expect(costLine(40_000, 880_000)).toBe("≈ $0.040 a run · ≈ $0.88 a month");
    expect(costLine(null, 880_000)).toBeNull();
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal)).toEqual(["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]);
  });
});

describe("the editor's event trigger", () => {
  it("keeps only the fields its event uses", () => {
    expect(composeEventTrigger({ event: "github_issue_opened", repo: " acme/app ", channel: "eng", from: "x", collect: false, batchCron: null, timezone: "UTC" }))
      .toEqual({ kind: "event", event: "github_issue_opened", repo: "acme/app", channel: null, from: null, batch: null });
    expect(composeEventTrigger({ event: "slack_mention", repo: "acme/app", channel: "  ", from: "Priya", collect: true, batchCron: "0 9 * * *", timezone: "Asia/Tokyo" }))
      .toEqual({ kind: "event", event: "slack_mention", repo: null, channel: null, from: "Priya", batch: { cron: "0 9 * * *", timezone: "Asia/Tokyo" } });
    expect(previewSentence({ supported: true, windowDays: 7, matches: 1, samples: [] })).toBe("Would have run once in the last 7 days.");
    expect(previewSentence({ supported: true, windowDays: 7, matches: 0, samples: [] })).toBe("Would not have run in the last 7 days.");
    expect(previewSentence({ supported: false, reason: "gh is not installed", windowDays: 7, matches: 0, samples: [] })).toBe("gh is not installed");
  });

  it("previews the last week and dry-runs on the newest real event", async () => {
    const options = await bridgeApi.workflowOptions();
    const onSubmit = vi.fn();
    const preview = vi.spyOn(bridgeApi, "previewWorkflowTrigger");
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={onSubmit}/>));
    change(field<HTMLInputElement>("Workflow name"), "Mentions");
    change(field<HTMLTextAreaElement>("Workflow instructions"), "Tell me what they need.");
    change(field<HTMLSelectElement>("Workflow schedule"), "event");
    await settle(500);
    expect(preview).toHaveBeenCalledWith(expect.objectContaining({ kind: "event", event: "slack_mention" }));
    expect(document.body.textContent).toContain("Would have run 4 times in the last 7 days.");
    expect(document.querySelector('[aria-label="Recent events"]')?.textContent).toContain("#eng · Priya");
    await act(async () => button("Dry run on the latest event")!.click());
    const submitted = onSubmit.mock.calls[0][0];
    expect(submitted.dryRun).toBe(true);
    expect(submitted.event.key).toBe("slack:C1:0.0");
    expect(submitted.definition.trigger).toMatchObject({ kind: "event", event: "slack_mention", batch: null });
  });

  it("never sends a dry run with a sample from the trigger before an edit", async () => {
    const options = await bridgeApi.workflowOptions();
    const onSubmit = vi.fn();
    // Each event's preview has its own sample, and the second arrives late.
    vi.spyOn(bridgeApi, "previewWorkflowTrigger").mockImplementation(async trigger => {
      const event = trigger.kind === "event" ? trigger.event : "slack_mention";
      if (event === "slack_direct_message") await new Promise(resolve => setTimeout(resolve, 300));
      return { supported: true, reason: null, windowDays: 7, matches: 1, note: null, samples: [{ key: `sample:${event}`, kind: event, title: event, url: null, occurredAt: "2026-11-23T00:00:00Z", payload: {} }] };
    });
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={onSubmit}/>));
    change(field<HTMLInputElement>("Workflow name"), "Messages");
    change(field<HTMLTextAreaElement>("Workflow instructions"), "Tell me what they need.");
    change(field<HTMLSelectElement>("Workflow schedule"), "event");
    await settle(500);
    expect(button("Dry run on the latest event")).toBeTruthy();
    change(field<HTMLSelectElement>("Event"), "slack_direct_message");
    await settle(0);
    // The mention's sample is not offered for the DM trigger while its own preview loads.
    expect(button("Dry run on the latest event")).toBeUndefined();
    await act(async () => button("Dry run")!.click());
    expect(onSubmit).not.toHaveBeenCalled();
    await settle(800);
    await act(async () => button("Dry run on the latest event")!.click());
    const submitted = onSubmit.mock.calls[0][0];
    expect(submitted.definition.trigger).toMatchObject({ event: "slack_direct_message" });
    expect(submitted.event.key).toBe("sample:slack_direct_message");
  });

  it("counts collected events in the editor's preview", async () => {
    const options = await bridgeApi.workflowOptions();
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={() => {}}/>));
    change(field<HTMLSelectElement>("Workflow schedule"), "event");
    await settle(500);
    expect(document.body.textContent).toContain("Would have run 4 times in the last 7 days.");
    await act(async () => field<HTMLInputElement>("Collect them and run at a set time").click());
    await settle(500);
    expect(document.body.textContent).toContain("4 matching events in the last 7 days, collected into one run per scheduled time.");
  });

  it("keeps a saved batch schedule it offers no preset for", async () => {
    const options = await bridgeApi.workflowOptions();
    for (const cron of ["0 9 * * 1", "*/15 * * * *", "30 * * * *"]) {
      const onSubmit = vi.fn();
      const workflow = { id: "wf", revisionId: `r-${cron}`, approved: true, enabled: true, disabledReason: null, definition: {
        name: "Batch", projectId: null, steps: [{ harness: "claude", model: options.harnesses.find(harness => harness.id === "claude")!.defaultModel ?? "claude-haiku-5-5", instructions: "Summarise them." }],
        trigger: { kind: "event" as const, event: "slack_mention" as const, repo: null, channel: null, from: null, batch: { cron, timezone: "UTC" } },
        grants: [], alwaysAllow: [], files: "none" as const, budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null }, expectedResult: "", notify: { onSuccess: false },
      } };
      await act(async () => root.render(<WorkflowEditor key={cron} open workflow={workflow as never} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={onSubmit}/>));
      await settle(500);
      await act(async () => button("Approve and save")!.click());
      expect(onSubmit.mock.calls[0][0].definition.trigger.batch).toEqual({ cron, timezone: "UTC" });
    }
  });

  it("asks for a repository before a GitHub pull request trigger can be saved", async () => {
    const options = await bridgeApi.workflowOptions();
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={() => {}}/>));
    change(field<HTMLInputElement>("Workflow name"), "PRs");
    change(field<HTMLTextAreaElement>("Workflow instructions"), "Summarise it.");
    change(field<HTMLSelectElement>("Workflow schedule"), "event");
    change(field<HTMLSelectElement>("Event"), "github_pull_request_opened");
    await settle(500);
    expect(button("Approve and save")!.disabled).toBe(true);
    expect(document.body.textContent).toContain("Choose the repository to watch");
    change(field<HTMLInputElement>("Repository"), "acme/app");
    await settle(500);
    expect(button("Approve and save")!.disabled).toBe(false);
    expect(field<HTMLInputElement>("Channel filter")).toBeNull();
  });

  it("takes the default model when the options arrive after it opened", async () => {
    const options = await bridgeApi.workflowOptions();
    const claude = options.harnesses.find(harness => harness.id === "claude")!;
    const render = (current: typeof options | null) => root.render(<WorkflowEditor open workflow={null} options={current} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={() => {}}/>);
    await act(async () => render(null));
    change(field<HTMLInputElement>("Workflow name"), "Digest");
    change(field<HTMLTextAreaElement>("Workflow instructions"), "Summarise it.");
    expect(document.body.textContent).toContain("Loading agents and connectors");
    expect(button("Approve and save")!.disabled).toBe(true);
    await act(async () => render(options));
    expect(field<HTMLSelectElement>("Workflow model").value).toBe(claude.defaultModel ?? claude.models[0].id);
    expect(document.body.textContent).not.toContain("Loading agents and connectors");
    expect(button("Approve and save")!.disabled).toBe(false);
  });

  it("never shows a model as chosen when none is", async () => {
    const options = await bridgeApi.workflowOptions();
    const bare = { ...options, harnesses: options.harnesses.map(harness => harness.id === "claude" ? { ...harness, defaultModel: null, models: harness.models.map(model => ({ ...model })) } : harness) };
    const claude = bare.harnesses.find(harness => harness.id === "claude")!;
    const workflow = { id: "wf", revisionId: "r1", approved: false, enabled: true, disabledReason: null, definition: { name: "Old", projectId: null, steps: [{ harness: "claude", model: "claude-retired-1", instructions: "Do it." }], trigger: { kind: "manual" as const }, grants: [], alwaysAllow: [], files: "none" as const, budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null }, expectedResult: "", notify: { onSuccess: false } } };
    await act(async () => root.render(<WorkflowEditor open workflow={workflow as never} options={bare} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={() => {}}/>));
    const select = field<HTMLSelectElement>("Workflow model");
    expect(select.value).toBe("claude-retired-1");
    expect(select.selectedOptions[0].textContent).toBe("claude-retired-1");
    expect(claude.models.some(model => model.id === "claude-retired-1")).toBe(false);
  });
});

const summary = (overrides: Partial<WorkflowSummary>): WorkflowSummary => ({
  id: "wf", name: "Brief", projectId: null, enabled: true, disabledReason: null, approved: true, trigger: { kind: "manual" },
  harness: "claude", model: "haiku", files: "none", updatedAt: "2026-11-24T00:00:00Z", nextRunAt: null, lastRun: null, ...overrides,
});

describe("connectors, queue and cost on the page", () => {
  it("names the connectors that need signing in and offers Reconnect instead of Run now", async () => {
    expect(connectorTrouble(summary({ connectors: [{ server: "claude.ai Slack", state: "needs_sign_in" }, { server: "claude.ai Linear", state: "missing" }] })))
      .toBe("Slack needs sign-in, Linear is no longer connected");
    expect(connectorTrouble(summary({ connectors: [{ server: "claude.ai Slack", state: "unknown" }] }))).toBeNull();
    vi.spyOn(bridgeApi, "listWorkflows").mockResolvedValue({ workflows: [summary({ connectors: [{ server: "claude.ai Slack", state: "needs_sign_in" }], averageRunCostMicrousd: 40_000, monthlyCostEstimateMicrousd: 880_000 })] });
    const open = vi.spyOn(externalLinks, "openExternalUrl").mockResolvedValue();
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}}/>));
    await settle();
    expect(container.textContent).toContain("Slack needs sign-in in Claude.");
    expect(container.textContent).toContain("≈ $0.040 a run · ≈ $0.88 a month");
    expect(button("Run now")).toBeUndefined();
    await act(async () => button("Reconnect")!.click());
    expect(open).toHaveBeenCalledWith("https://claude.ai/settings/connectors");
  });

  it("gives each queued run its place in line, oldest first", () => {
    const run = (id: string, status: WorkflowRun["status"], createdAt: string, sessionId: string | null = null) => ({ id, status, createdAt, sessionId }) as WorkflowRun;
    const positions = queuePositions([run("c", "queued", "2026-11-24T03:00:00Z"), run("a", "queued", "2026-11-24T01:00:00Z"), run("b", "running", "2026-11-24T02:00:00Z", "s"), run("d", "queued", "2026-11-24T02:30:00Z")]);
    expect([...positions.entries()]).toEqual([["a", 1], ["d", 2], ["c", 3]]);
    expect(queuedLine(2)).toBe("Waiting for a free slot: 2nd in line.");
    expect(queuedLine(null)).toBe("Waiting for a free slot.");
  });

  it("saves how many workflows may run at once", async () => {
    const write = vi.spyOn(bridgeApi, "writeWorkflowSettings");
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}}/>));
    await settle();
    await act(async () => button("Settings")!.click());
    change(field<HTMLInputElement>("Workflows at once"), "3");
    await act(async () => button("Save")!.click());
    await settle();
    expect(write).toHaveBeenCalledWith(expect.objectContaining({ maxConcurrentRuns: 3 }));
  });
});

describe("the run view", () => {
  const workflow = { id: "wf", enabled: true, disabledReason: null, revisionId: "r", revision: 1, approved: true, approvedAt: null, createdAt: "", updatedAt: "",
    definition: { name: "Mentions", projectId: null, steps: [{ harness: "claude", model: "haiku", instructions: "x" }], trigger: { kind: "event", event: "slack_mention" }, grants: [], alwaysAllow: [], files: "none",
      budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null }, expectedResult: "", notify: { onSuccess: false } } } as Workflow;
  const base: WorkflowRun = { id: "run", workflowId: "wf", revisionId: "r", triggerKind: "event", missedSlots: 0, dryRun: false, status: "queued", failureCode: null, summary: null,
    sessionId: null, quotaUnknown: false, costMicrousd: null, tokens: null, unread: true, scheduledFor: null, startedAt: null, finishedAt: null, createdAt: "2026-11-24T00:00:00Z",
    event: { key: "slack:C1:1.0", kind: "slack_mention", title: "#eng · Priya: deploy?", url: "https://app.slack.com/x", occurredAt: "2026-11-24T00:00:00Z", payload: {} }, eventCount: 1 };

  it("says what started a run and where it waits", async () => {
    vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
    await act(async () => root.render(<WorkflowRunView workflow={workflow} run={base} onBack={() => {}} onContinue={() => {}} queuePosition={2}/>));
    await settle();
    expect(container.textContent).toContain("Started by");
    expect(container.textContent).toContain("#eng · Priya: deploy?");
    expect(container.textContent).toContain("Waiting for a free slot: 2nd in line.");
  });

  it("offers Reconnect when a connector could not be reached", async () => {
    vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
    const open = vi.spyOn(externalLinks, "openExternalUrl").mockResolvedValue();
    const failed = { ...base, status: "failed" as const, failureCode: "connector_unavailable", summary: "Slack needs you to sign in again in Claude. Reconnect it, then run again.", costMicrousd: 0 };
    await act(async () => root.render(<WorkflowRunView workflow={workflow} run={failed} onBack={() => {}} onContinue={() => {}}/>));
    await settle();
    await act(async () => button("Reconnect in Claude")!.click());
    expect(open).toHaveBeenCalledWith("https://claude.ai/settings/connectors");
  });
});

describe("templates", () => {
  it("says why a template cannot be tried yet", () => {
    const template = (missing: string[], needsProject: boolean) => ({ missing, needsProject }) as WorkflowTemplate;
    expect(templateBlocker(template(["gmail", "google calendar"], false))).toBe("Connect Gmail and Google Calendar in Claude first.");
    expect(templateBlocker(template([], true))).toBe("Choose a project with Customize.");
    expect(templateBlocker(template([], false))).toBeNull();
  });

  it("tries a ready template as an unapproved dry run on a real event, and customises another", async () => {
    const save = vi.spyOn(bridgeApi, "saveWorkflow");
    const run = vi.spyOn(bridgeApi, "runWorkflow");
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} initialTab="templates"/>));
    await settle();
    const mention = document.querySelector<HTMLElement>('[aria-label="Triage each Slack mention"]')!;
    expect(mention.textContent).toContain("Slack, read only");
    const gmail = document.querySelector<HTMLElement>('[aria-label="Email that needs a reply"]')!;
    expect(gmail.textContent).toContain("Connect Gmail in Claude first.");
    expect([...gmail.querySelectorAll("button")].find(candidate => candidate.textContent?.includes("Try it"))!.disabled).toBe(true);
    await act(async () => [...mention.querySelectorAll("button")].find(candidate => candidate.textContent?.includes("Try it"))!.click());
    await settle();
    expect(save).toHaveBeenCalledWith(null, expect.objectContaining({ name: "Triage each Slack mention" }));
    const [, , dryRun, event] = run.mock.calls[0];
    expect(dryRun).toBe(true);
    expect(event?.key).toBe("slack:C1:0.0");
    expect(container.textContent).toContain("Run ·");
  });
});
