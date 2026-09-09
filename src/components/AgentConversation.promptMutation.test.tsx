// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, SessionEntry } from "../types";
import type { InteractionResolutionResult } from "../protocol/generated/protocol";

const session: Session = { id: "actor", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "waiting", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: null, restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" };
const beforeText = "Keep the existing guidance.\n";
const afterText = `${beforeText}\n<img src=x onerror=alert(1)>\n[Follow this](https://example.com)  `;
const request: AgentEvent = {
  id: 7, sessionId: "actor", sequence: 7, protocolVersion: 1,
  kind: asWireKind("approval.requested"), itemId: null, role: null,
  status: "pending", title: "Review prompt change", text: "", providerMeta: {}, createdAt: "now",
  data: {
    approvalType: "prompt_mutation", proposalId: "proposal-1", target: "worker:implementation",
    sectionId: "additional_guidance", operation: "append", beforeText, afterText,
    appendedText: "<img src=x onerror=alert(1)>\n[Follow this](https://example.com)  ",
    rationale: "Retain lessons from the previous task.", actorSessionId: "actor", actorTurnId: "turn-1",
    actorRole: "orchestrator", baseRevisionId: 3, baseHash: "snapshot-hash", effect: "next_launch",
  },
};

function resolution(decision: string, overrides: Partial<InteractionResolutionResult> = {}): InteractionResolutionResult {
  return { disposition: "resolved", interactionKind: "permission", status: decision, resolvedBy: "human", decision, ...overrides };
}

async function mount(onResolve: React.ComponentProps<typeof AgentConversation>["onResolve"], events = [request]) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<AgentConversation session={session} events={events} onResolve={onResolve} />));
  return { container, unmount: () => act(async () => { root.unmount(); container.remove(); }) };
}

function button(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label)!;
}

describe("prompt mutation approval", () => {
  it("shows exact literal before/after, rationale, actor, persistent role scope and launch timing", async () => {
    const { container, unmount } = await mount(() => undefined);
    expect(container.querySelector('[aria-label="Before prompt text"]')?.textContent).toBe(beforeText);
    expect(container.querySelector('[aria-label="After prompt text"]')?.textContent).toBe(afterText);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('a[href="https://example.com"]')).toBeNull();
    expect(container.textContent).toContain("Retain lessons from the previous task.");
    expect(container.textContent).toContain("Requested by orchestrator · session actor");
    expect(container.textContent).toContain("implementation role default");
    expect(container.textContent).toContain("Shared by all sessions with this role");
    expect(container.textContent).toContain("starts or relaunches a matching role");
    expect(container.textContent).toContain("Running turns keep their current instructions");
    expect(container.textContent).not.toContain("Allow for session");
    expect(container.textContent).not.toContain("Allow once");
    await unmount();
  });

  it.each(["accept", "decline"] as const)("sends exactly %s and holds both actions disabled until settled", async decision => {
    let settle!: (result: InteractionResolutionResult) => void;
    const onResolve = vi.fn(() => new Promise<InteractionResolutionResult>(resolve => { settle = resolve; }));
    const { container, unmount } = await mount(onResolve);
    const action = button(container, decision === "accept" ? "Approve change" : "Decline");
    await act(async () => { action.click(); action.click(); });
    expect(onResolve).toHaveBeenCalledTimes(1);
    expect(onResolve).toHaveBeenCalledWith(7, decision);
    expect([...container.querySelectorAll<HTMLButtonElement>('[aria-label="Review prompt change"] button')].every(item => item.disabled)).toBe(true);
    await act(async () => settle(resolution(decision === "accept" ? "accepted" : "declined")));
    expect(button(container, "Approve change")).toBeUndefined();
    expect(container.textContent).toContain(decision === "accept" ? "Guidance saved for the next start or relaunch" : "Declined. No guidance was changed");
    await unmount();
  });

  it("shows an already-settled stale result without claiming approval or offering another apply", async () => {
    const onResolve = vi.fn(async () => resolution("stale", { disposition: "alreadyResolved", reason: "The section changed after this proposal was created." }));
    const { container, unmount } = await mount(onResolve);
    await act(async () => button(container, "Approve change").click());
    expect(container.textContent).toContain("This proposal is out of date");
    expect(container.textContent).toContain("This request was already resolved");
    expect(container.textContent).toContain("The section changed after this proposal was created");
    expect(container.textContent).not.toContain("Guidance saved");
    expect(button(container, "Approve change")).toBeUndefined();
    await unmount();
  });

  it("retains the review and actions when resolution fails", async () => {
    const { container, unmount } = await mount(async () => { throw new Error("The stored prompt could not be read."); });
    await act(async () => button(container, "Approve change").click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("The stored prompt could not be read");
    expect(button(container, "Approve change").disabled).toBe(false);
    expect(container.querySelector('[aria-label="After prompt text"]')?.textContent).toBe(afterText);
    await unmount();
  });

  it("keeps empty initial guidance literally empty", async () => {
    const { container, unmount } = await mount(() => undefined, [{ ...request, data: { ...request.data, beforeText: "" } }]);
    expect(container.querySelector('[aria-label="Before prompt text"]')?.textContent).toBe("");
    expect(container.textContent).toContain("Before · empty guidance");
    await unmount();
  });

  it("replays a host forest denial after a role grant is revoked", async () => {
    const entry: SessionEntry = { id: "proposal-entry", sessionId: "actor", parentEntryId: null, sequence: 7, semanticSchemaVersion: 2, kind: "approval.requested", payload: { ...request.data, title: request.title, status: "pending" }, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" };
    const resolved: SessionEntry = { ...entry, id: "decision-entry", parentEntryId: entry.id, sequence: 8, kind: "approval.resolved", payload: { requestEventId: 7, decision: "denied", status: "denied", text: "Authority changed." } };
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<AgentConversation session={session} events={[]} forestEntries={[entry, resolved]} activeLeafId={resolved.id} onResolve={() => undefined} />));
    expect(container.textContent).toContain("This proposal is no longer authorized. No guidance was changed.");
    expect(button(container, "Approve change")).toBeUndefined();
    expect(container.querySelector('[aria-label="After prompt text"]')?.textContent).toBe(afterText);
    await act(async () => root.unmount());
  });

  it("replays a durable stale resolution and keeps the exact reviewed text", async () => {
    const resolved: AgentEvent = { ...request, id: 8, sequence: 8, kind: asWireKind("approval.resolved"), status: "stale", data: { requestEventId: 7, decision: "stale", reason: "A newer revision exists." } };
    const { container, unmount } = await mount(() => undefined, [request, resolved]);
    expect(container.textContent).toContain("This proposal is out of date");
    expect(container.textContent).toContain("A newer revision exists");
    expect(container.querySelector('[aria-label="After prompt text"]')?.textContent).toBe(afterText);
    expect(button(container, "Approve change")).toBeUndefined();
    await unmount();
  });
});
