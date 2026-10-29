// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { WorkflowRun } from "../../protocol/generated/protocol";
import { asWireKind } from "../../transcript/wire";
import type { AgentEvent } from "../../types";
import { WorkflowInbox, WorkflowRunsStrip, orderResults } from "./WorkflowInbox";
import { needsYouCount, workflowNotification } from "./notify";

const run = (id: string, status: WorkflowRun["status"], extra: Partial<WorkflowRun> = {}): WorkflowRun => ({
  id, workflowId: "wf-1", revisionId: "r1", triggerKind: "manual", missedSlots: 0, dryRun: false, status,
  failureCode: null, summary: null, sessionId: `session-${id}`, quotaUnknown: false, costMicrousd: null, tokens: null,
  unread: true, scheduledFor: null, startedAt: null, finishedAt: null, createdAt: "2026-10-29T09:00:00Z", ...extra,
});

let container: HTMLDivElement;
let root: Root;
beforeEach(() => { container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container); });
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
const button = (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim().startsWith(label));

describe("workflowNotification", () => {
  it("notifies each run and status once, and successes only when asked", () => {
    const seen = new Set<string>();
    expect(workflowNotification(run("a", "waiting_approval"), "Release notes", false, seen)?.title).toBe("Release notes needs you");
    expect(workflowNotification(run("a", "waiting_approval"), "Release notes", false, seen)).toBeNull();
    expect(workflowNotification(run("a", "failed", { summary: "Slack was down" }), "Release notes", false, seen)?.body).toBe("Slack was down");
    expect(workflowNotification(run("b", "succeeded"), "Morning brief", false, seen)).toBeNull();
    expect(workflowNotification(run("c", "succeeded"), "Morning brief", true, seen)?.title).toBe("Morning brief finished");
    expect(workflowNotification(run("d", "running"), "Morning brief", true, seen)).toBeNull();
    expect(workflowNotification(run("e", "unverified"), "Morning brief", false, seen)?.title).toBe("Morning brief needs a look");
  });

  it("counts what waits on the person", () => {
    expect(needsYouCount([run("a", "waiting_approval", { unread: false }), run("b", "failed"), run("c", "succeeded", { unread: false }), run("d", "running")])).toBe(2);
  });
});

describe("WorkflowInbox", () => {
  it("orders results newest first and unread first at the same moment", () => {
    const ordered = orderResults([
      run("old", "succeeded", { createdAt: "2026-10-28T09:00:00Z" }),
      run("read", "succeeded", { unread: false }),
      run("unread", "failed"),
    ]);
    expect(ordered.map(candidate => candidate.id)).toEqual(["unread", "read", "old"]);
  });

  it("answers a waiting run's card in place and opens a result", async () => {
    const event = { sequence: 7, kind: asWireKind("permission.requested"), status: "pending", role: null, text: null, title: null, providerMeta: {},
      data: { tool_name: "mcp__claude_ai_Slack__slack_send_message" } } as unknown as AgentEvent;
    vi.spyOn(bridgeApi, "replaySessionEvents").mockResolvedValue([event]);
    const resolve = vi.spyOn(bridgeApi, "resolveApproval").mockResolvedValue({} as never);
    const opened = vi.fn();
    const changed = vi.fn();
    const runs = [run("w", "waiting_approval"), run("r", "succeeded", { summary: "Posted the notes." })];
    await act(async () => root.render(<WorkflowInbox runs={runs} names={new Map([["wf-1", "Release notes"]])} onOpenRun={opened} onChanged={changed}/>));
    await settle();
    expect(container.textContent).toContain("Release notes wants to change something");
    expect(container.textContent).toContain("Slack · slack_send_message");
    await act(async () => button("Decline")!.click());
    expect(resolve).toHaveBeenCalledWith("session-w", 7, "decline");
    await act(async () => button("Allow once")!.click());
    expect(resolve).toHaveBeenLastCalledWith("session-w", 7, "accept");
    expect(changed).toHaveBeenCalledTimes(2);
    await act(async () => button("Release notes · succeeded")!.click());
    expect(opened).toHaveBeenCalledWith(runs[1]);
  });

  it("clears unread on open through the screen", () => {
    const screen = readFileSync(resolve(process.cwd(), "src/components/WorkflowsScreen.tsx"), "utf8");
    expect(screen).toContain("bridgeApi.markWorkflowRunsRead([run.id])");
  });
});

describe("WorkflowRunsStrip", () => {
  it("lists active runs on Mission Control and opens one", async () => {
    const opened = vi.fn();
    const runs = [run("a", "running"), run("b", "succeeded"), run("c", "waiting_approval")];
    await act(async () => root.render(<WorkflowRunsStrip runs={runs} names={new Map([["wf-1", "Morning brief"]])} onOpenRun={opened}/>));
    expect(container.querySelectorAll("button")).toHaveLength(2);
    await act(async () => button("Morning brief · running")!.click());
    expect(opened).toHaveBeenCalledWith(runs[0]);
    const mission = readFileSync(resolve(process.cwd(), "src/components/MissionControl.tsx"), "utf8");
    expect(mission).toContain("<WorkflowRunsStrip");
  });
});
