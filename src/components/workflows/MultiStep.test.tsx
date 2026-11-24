// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { Workflow, WorkflowDefinition, WorkflowRun } from "../../protocol/generated/protocol";
import { WorkflowEditor, stepFiles, stepRefusal, stepServers, stepsProblem, type EditorSubmit } from "./WorkflowEditor";
import { WorkflowRunView, runStepRows } from "./WorkflowRunView";

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
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim().includes(label) || candidate.getAttribute("aria-label") === label);
const field = <T extends HTMLElement>(label: string) => document.querySelector<T>(`[aria-label="${label}"]`);
function change(element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}
const click = (element: HTMLElement | null | undefined) => act(() => { element!.click(); });

/** Claude reads Slack, Codex fixes the code in the worktree, Claude writes Linear. */
function threeSteps(): WorkflowDefinition {
  return {
    name: "Fix what Slack asks",
    projectId: "proj-1",
    steps: [
      { harness: "claude", model: "haiku", instructions: "Read the bug reports.", servers: ["claude.ai Slack"], files: "none" },
      { harness: "codex", model: "gpt-5.5", instructions: "Fix them.", servers: [], files: "worktree" },
      { harness: "claude", model: "sonnet", instructions: "File what changed.", servers: ["claude.ai Linear"], files: "none" },
    ],
    trigger: { kind: "manual" },
    grants: [
      { server: "claude.ai Slack", read: true, write: "never" },
      { server: "claude.ai Linear", read: true, write: "ask" },
    ],
    alwaysAllow: [],
    files: "worktree",
    budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null },
    expectedResult: "",
    notify: { onSuccess: false },
  };
}

describe("a workflow's steps", () => {
  it("give each step its own share, and say what is left over", async () => {
    const options = await bridgeApi.workflowOptions();
    const workflow = threeSteps();
    expect(stepServers(workflow, 0)).toEqual(["claude.ai Slack"]);
    expect(stepFiles(workflow, 1)).toBe("worktree");
    expect(stepRefusal(options, workflow, 1)).toBeNull();
    expect(stepRefusal(options, workflow, 0, "codex")).toMatch(/per-tool authority/);
    expect(stepsProblem(workflow)).toBeNull();
    expect(stepsProblem({ ...workflow, steps: workflow.steps.map((step, index) => index === 2 ? { ...step, servers: [] } : step) })).toBe("No step uses claude.ai Linear. Give it to a step, or remove it.");
    expect(stepsProblem({ ...workflow, steps: workflow.steps.map((step, index) => index === 1 ? { ...step, servers: ["claude.ai Slack"] } : step) })).toBe("Step 2 works in the worktree, so it cannot use connectors.");
    expect(stepsProblem({ ...workflow, steps: workflow.steps.map(step => ({ ...step, files: "none" })) })).toMatch(/No step works in the worktree/);
    // One step is never a problem here: its own rules stay as they were.
    expect(stepsProblem({ ...workflow, steps: [workflow.steps[0]] })).toBeNull();
    expect(stepServers({ ...workflow, steps: [{ ...workflow.steps[0], servers: null }] }, 0)).toEqual(["claude.ai Slack", "claude.ai Linear"]);
  });

  it("add, fill and remove steps in the editor, up to five", async () => {
    const options = await bridgeApi.workflowOptions();
    const submitted: EditorSubmit[] = [];
    await act(async () => root.render(<WorkflowEditor open workflow={null} options={options} projects={[]} busy={false} error={null} onCancel={() => {}} onSubmit={submit => submitted.push(submit)}/>));
    change(field<HTMLInputElement>("Workflow name")!, "Two steps");
    change(field<HTMLTextAreaElement>("Workflow instructions")!, "Read my Slack mentions.");
    await click(field<HTMLInputElement>("Read claude.ai Slack"));
    expect(field("Step 2 agent")).toBeNull();

    await click(button("Add a step"));
    expect(field<HTMLSelectElement>("Step 2 agent")!.value).toBe("claude");
    expect(button("Approve and save")!.disabled).toBe(true);
    change(field<HTMLTextAreaElement>("Step 2 instructions")!, "Summarise them.");
    change(field<HTMLSelectElement>("Step 2 model")!, "sonnet");
    await settle();
    expect(button("Approve and save")!.disabled).toBe(false);
    // Step 1 keeps Slack; step 2 has nothing until it is given something.
    expect(field<HTMLInputElement>("Step 1 uses claude.ai Slack")!.checked).toBe(true);
    expect(field<HTMLInputElement>("Step 2 uses claude.ai Slack")!.checked).toBe(false);
    // Codex cannot run a step with no files: it is offered with the reason.
    const codex = [...field<HTMLSelectElement>("Step 2 agent")!.options].find(option => option.value === "codex")!;
    expect(codex.disabled).toBe(true);
    expect(codex.textContent).toMatch(/per-tool authority/);

    for (let added = 2; added < 5; added += 1) await click(button("Add a step"));
    expect(field("Step 5 agent")).not.toBeNull();
    expect(button("Add a step")).toBeUndefined();
    await click(button("Remove step 5"));
    await click(button("Remove step 4"));
    await click(button("Remove step 3"));
    expect(field("Step 3 agent")).toBeNull();

    await click(button("Approve and save"));
    const saved = submitted.at(-1)!.definition;
    expect(saved.steps).toHaveLength(2);
    expect(saved.steps[1]).toMatchObject({ harness: "claude", model: "sonnet", instructions: "Summarise them.", servers: [], files: "none" });
    expect(saved.steps[0].servers ?? null).toBeNull();
  });

  it("let a step work in the worktree on Codex, with no connectors", async () => {
    const options = await bridgeApi.workflowOptions();
    const projects = [{ id: "proj-1", name: "app", path: "/repo/app" }] as never;
    const workflow = { id: "wf-1", enabled: true, disabledReason: null, revisionId: "r1", revision: 1, approved: true, approvedAt: null, definition: threeSteps(), createdAt: "now", updatedAt: "now" } as Workflow;
    const submitted: EditorSubmit[] = [];
    await act(async () => root.render(<WorkflowEditor open workflow={workflow} options={options} projects={projects} busy={false} error={null} onCancel={() => {}} onSubmit={submit => submitted.push(submit)}/>));
    expect(field<HTMLSelectElement>("Step 2 agent")!.value).toBe("codex");
    expect(field<HTMLInputElement>("Step 2 works in the worktree")!.checked).toBe(true);
    expect(field<HTMLInputElement>("Step 2 uses claude.ai Slack")!.disabled).toBe(true);
    expect(document.body.textContent).not.toMatch(/cannot run this step/);
    expect(button("Approve and save")!.disabled).toBe(false);

    // Out of the worktree, Codex cannot run step 2, and Save waits.
    await click(field<HTMLInputElement>("Step 2 works in the worktree"));
    expect(document.body.textContent).toMatch(/Codex cannot run this step/);
    expect(button("Approve and save")!.disabled).toBe(true);
    await click(field<HTMLInputElement>("Step 2 works in the worktree"));

    // Taking Linear from step 3 leaves it unused.
    await click(field<HTMLInputElement>("Step 3 uses claude.ai Linear"));
    expect(document.body.textContent).toContain("No step uses claude.ai Linear.");
    expect(button("Approve and save")!.disabled).toBe(true);
    await click(field<HTMLInputElement>("Step 3 uses claude.ai Linear"));

    // Ungranting Slack takes it out of step 1 too.
    await click(field<HTMLInputElement>("Read claude.ai Slack"));
    await click(button("Approve and save"));
    const saved = submitted.at(-1)!.definition;
    expect(saved.grants?.map(grant => grant.server)).toEqual(["claude.ai Linear"]);
    expect(saved.steps[0].servers).toEqual([]);
  });

  it("list a run's steps, and open each one's own transcript", async () => {
    const workflow = { id: "wf-1", enabled: true, disabledReason: null, revisionId: "r1", revision: 1, approved: true, approvedAt: null, definition: threeSteps(), createdAt: "now", updatedAt: "now" } as Workflow;
    const step = (index: number, harness: string, status: WorkflowRun["status"], summary: string | null) => ({
      index, harness, model: "m", status, sessionId: `s-${index}`, failureCode: null, summary, tokens: null, costMicrousd: null, startedAt: "now", finishedAt: null,
    });
    const run = {
      id: "run-1", workflowId: "wf-1", revisionId: "r1", triggerKind: "manual", missedSlots: 0, dryRun: false, status: "running",
      failureCode: null, summary: null, sessionId: "s-1", quotaUnknown: false, costMicrousd: null, tokens: null, unread: true,
      scheduledFor: null, startedAt: new Date().toISOString(), finishedAt: null, createdAt: new Date().toISOString(), event: null, eventCount: 0,
      steps: [step(0, "claude", "succeeded", "Two bugs: login and export."), step(1, "codex", "running", null)],
    } as WorkflowRun;
    expect(runStepRows(workflow, run).map(row => [row.agent, row.status, row.sessionId])).toEqual([
      ["claude", "succeeded", "s-0"],
      ["codex", "running", "s-1"],
      ["claude", "not started yet", null],
    ]);
    expect(runStepRows(workflow, { ...run, status: "failed" })[2].status).toBe("did not start");

    const replay = vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
    await act(async () => root.render(<WorkflowRunView workflow={workflow} run={run} onBack={() => {}} onContinue={() => {}}/>));
    await settle();
    const list = document.querySelector('[aria-label="Steps"]')!;
    expect(list.textContent).toContain("Two bugs: login and export.");
    expect(list.textContent).toContain("Codex");
    expect(replay).toHaveBeenLastCalledWith("s-1", 0, 1000);
    const rows = [...list.querySelectorAll("button")];
    expect(rows[1].getAttribute("aria-pressed")).toBe("true");
    expect(rows[2].disabled).toBe(true);
    await click(rows[0]);
    await settle();
    expect(replay).toHaveBeenLastCalledWith("s-0", 0, 1000);
    expect(rows[0].getAttribute("aria-pressed")).toBe("true");
  });

  it("show a single-step run as it always was", async () => {
    const workflow = { id: "wf-1", enabled: true, disabledReason: null, revisionId: "r1", revision: 1, approved: true, approvedAt: null, definition: { ...threeSteps(), steps: [threeSteps().steps[0]] }, createdAt: "now", updatedAt: "now" } as Workflow;
    const run = { id: "run-1", workflowId: "wf-1", revisionId: "r1", triggerKind: "manual", missedSlots: 0, dryRun: false, status: "succeeded", failureCode: null, summary: "Done.", sessionId: "s-0", quotaUnknown: false, costMicrousd: null, tokens: null, unread: false, scheduledFor: null, startedAt: null, finishedAt: null, createdAt: new Date().toISOString(), event: null, eventCount: 0, steps: [] } as WorkflowRun;
    vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([]);
    await act(async () => root.render(<WorkflowRunView workflow={workflow} run={run} onBack={() => {}} onContinue={() => {}}/>));
    await settle();
    expect(document.querySelector('[aria-label="Steps"]')).toBeNull();
  });
});
