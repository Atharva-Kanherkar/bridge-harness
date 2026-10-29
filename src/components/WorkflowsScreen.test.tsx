// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ForkSessionResult, Workflow, WorkflowOptions, WorkflowRun } from "../protocol/generated/protocol";
import { WorkflowsScreen } from "./WorkflowsScreen";
import { WorkflowRunView, runSteps } from "./workflows/WorkflowRunView";
import { harnessRefusal } from "./workflows/WorkflowEditor";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";

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

const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim().includes(label));

describe("WorkflowsScreen", () => {
  it("replaces Scheduled tasks in the sidebar", () => {
    const sidebar = readFileSync(resolve(process.cwd(), "src/components/BridgeSidebar.tsx"), "utf8");
    expect(sidebar).toContain('label="Workflows"');
    expect(sidebar).not.toContain("Scheduled tasks");
  });

  it("lists a workflow without a project as No project", async () => {
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} />));
    await settle();
    expect(container.textContent).toContain("Morning brief");
    expect(container.textContent).toContain("No project");
    expect(container.textContent).toContain("Weekdays at 09:00");
  });

  it("keeps native schedules, with their actions, in their own section", async () => {
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} initialTab="native" />));
    await settle();
    expect(container.textContent).toContain("Schedules your coding agents keep themselves");
    expect(container.textContent).toContain("Refresh");
  });

  it("shows the empty state when there are no workflows", async () => {
    vi.spyOn(bridgeApi, "listWorkflows").mockResolvedValue({ workflows: [] });
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} />));
    await settle();
    expect(container.textContent).toContain("Automate a job you do every day");
    expect(container.textContent).toContain("No project needed");
  });

  it("creates a no-project workflow without choosing a folder", async () => {
    const save = vi.spyOn(bridgeApi, "saveWorkflow");
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} />));
    await settle();
    await act(async () => button("New workflow")!.click());
    await settle();
    const field = (label: string) => document.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[aria-label="${label}"]`)!;
    const type = (element: HTMLInputElement | HTMLTextAreaElement, value: string) => {
      const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), "value")!.set!;
      setter.call(element, value);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    };
    await act(async () => type(field("Workflow name"), "Release notes"));
    await act(async () => type(field("Workflow instructions"), "Write release notes from merged PRs."));
    expect((document.querySelector<HTMLSelectElement>('[aria-label="Workflow project"]'))!.value).toBe("");
    await act(async () => button("Approve and save")!.click());
    await settle();
    expect(save).toHaveBeenCalledTimes(1);
    const [, definition] = save.mock.calls[0];
    expect(definition.projectId).toBeNull();
    expect(definition.files).toBe("none");
    expect(definition.name).toBe("Release notes");
  });
});

describe("drafting", () => {
  it("opens a prefilled editor from a sentence and saves nothing until confirmed", async () => {
    const draft = vi.spyOn(bridgeApi, "draftWorkflowFromText");
    const save = vi.spyOn(bridgeApi, "saveWorkflow");
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} />));
    await settle();
    const input = document.querySelector<HTMLInputElement>('[aria-label="Describe a job"]')!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    await act(async () => { setter.call(input, "Every Friday summarise my invoices"); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => button("Draft it")!.click());
    await settle();
    expect(draft).toHaveBeenCalledWith("Every Friday summarise my invoices");
    expect(document.body.textContent).toContain("Drafted workflow");
    expect(document.querySelector('[aria-label="Draft notes"]')?.textContent).toContain("Fill in the rest below");
    expect(save).not.toHaveBeenCalled();
  });

  it("opens a draft made elsewhere once, then hands it back", async () => {
    const opened = vi.fn();
    const { draft } = await bridgeApi.draftWorkflowFromText("Post release notes to #releases");
    await act(async () => root.render(<WorkflowsScreen projects={[]} onContinueInChat={() => {}} initialDraft={draft} onDraftOpened={opened} />));
    await settle();
    expect(document.body.textContent).toContain("Drafted workflow");
    expect(opened).toHaveBeenCalledTimes(1);
  });

  it("offers Save as workflow in the sidebar chat menu", () => {
    const sidebar = readFileSync(resolve(process.cwd(), "src/components/BridgeSidebar.tsx"), "utf8");
    const app = readFileSync(resolve(process.cwd(), "src/App.tsx"), "utf8");
    expect(sidebar).toContain("Save as workflow…");
    expect(app).toContain("onSaveChatAsWorkflow={chat => void saveChatAsWorkflow(chat)}");
  });
});

describe("harnessRefusal", () => {
  const options: WorkflowOptions = {
    harnesses: [
      { id: "claude", label: "Claude", available: true, connectorsAllowed: true, connectorsReason: null, worktreeAllowed: true, defaultModel: "haiku", models: [] },
      { id: "codex", label: "Codex", available: true, connectorsAllowed: false, connectorsReason: "no per-tool authority", worktreeAllowed: true, defaultModel: null, models: [] },
    ],
    servers: [],
  };

  it("disables a harness that cannot enforce tool identities, with the gate's reason", () => {
    expect(harnessRefusal(options, "codex", { files: "none", grants: [] })).toBe("no per-tool authority");
    expect(harnessRefusal(options, "codex", { files: "worktree", grants: [] })).toBeNull();
    expect(harnessRefusal(options, "claude", { files: "none", grants: [{ server: "s", read: true, write: "never" }] })).toBeNull();
  });
});

describe("WorkflowRunView", () => {
  const workflow = {
    id: "wf-1", enabled: true, disabledReason: null, revisionId: "r1", revision: 1, approved: true, approvedAt: null,
    createdAt: "2026-10-29T00:00:00Z", updatedAt: "2026-10-29T00:00:00Z",
    definition: { name: "Release notes", projectId: null, steps: [{ harness: "claude", model: "haiku", instructions: "x" }], trigger: { kind: "manual" }, grants: [], alwaysAllow: [], files: "none", budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null }, expectedResult: "", notify: { onSuccess: false } },
  } as Workflow;
  const run: WorkflowRun = {
    id: "run-1", workflowId: "wf-1", revisionId: "r1", triggerKind: "manual", missedSlots: 0, dryRun: false, status: "succeeded",
    failureCode: null, summary: "Posted the notes.", sessionId: "session-1", quotaUnknown: false, costMicrousd: null, tokens: 1200, unread: true,
    scheduledFor: null, startedAt: "2026-10-29T00:00:01Z", finishedAt: "2026-10-29T00:01:01Z", createdAt: "2026-10-29T00:00:00Z",
  };

  it("continues a run in a normal chat by forking it", async () => {
    vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
    vi.spyOn(bridgeApi, "sessionForest").mockResolvedValue({ entries: [{ id: "entry-1", sequence: 1 }, { id: "entry-2", sequence: 2 }] } as never);
    const fork = vi.spyOn(bridgeApi, "forkSession").mockResolvedValue({ sessionId: "chat-2" } as ForkSessionResult);
    const onContinue = vi.fn();
    await act(async () => root.render(<WorkflowRunView workflow={workflow} run={run} onBack={() => {}} onContinue={onContinue} />));
    await settle();
    expect(container.textContent).toContain("Posted the notes.");
    await act(async () => button("Continue in chat")!.click());
    await settle();
    expect(fork).toHaveBeenCalledWith("session-1", "entry-2", "Release notes (continued)", null, null, "shared");
    expect(onContinue).toHaveBeenCalledWith({ sessionId: "chat-2" });
  });

  it("offers the three answers only on a card Bridge has not decided", () => {
    const event = (sequence: number, kind: string, data: Record<string, unknown>, status = "pending") =>
      ({ sequence, kind: asWireKind(kind), data, status, role: null, text: null, title: null, providerMeta: {} }) as unknown as AgentEvent;
    const steps = runSteps([
      event(1, "permission.requested", { tool_name: "mcp__claude_ai_Slack__slack_send_message", resolvedBy: "workflow" }, "settling"),
      event(2, "permission.requested", { tool_name: "mcp__claude_ai_Slack__slack_delete_message" }),
    ]);
    expect(steps.map(step => Boolean(step.pending))).toEqual([false, true]);
    expect(steps[1].title).toBe("Slack · slack_delete_message");
    expect(steps[1].identity).toBe("mcp__claude_ai_Slack__slack_delete_message");
  });
});
