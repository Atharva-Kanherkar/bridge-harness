import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { MissionControl } from "./MissionControl";
import { asWireKind } from "../transcript/wire";

const NOW = Date.parse("2026-07-29T10:05:00Z");

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: "workspace-1",
  harness: "codex",
  label: id,
  status: "working",
  startedAt: "2026-07-29T10:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  kind: "worker",
  ...overrides,
});

const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId,
  parentSessionId: "orchestrator-1",
  lifecycleState: "working",
  taskFamily: "implementation",
  compatibilityKey: "implementation",
  resultStatus: "pending",
  retryCount: 0,
  warmUntil: null,
  worktreePath: null,
  worktreeBranch: null,
  lastResult: null,
  lastActivityAt: "2026-07-29T10:04:55Z",
  waitingSince: null,
  waitingReason: null,
  progressSummary: null,
  updatedAt: "2026-07-29T10:04:55Z",
  ...overrides,
});

const event = (id: number, sessionId: string, text: string): AgentEvent => ({
  id,
  sessionId,
  sequence: id,
  protocolVersion: 1,
  kind: asWireKind("reasoning"),
  itemId: null,
  role: "assistant",
  status: null,
  title: null,
  text,
  data: {},
  providerMeta: {},
  createdAt: "2026-07-29T10:04:55Z",
});

describe("MissionControl", () => {
  it("renders one window per live agent with its status and live stream", () => {
    const sessions = [
      session("orchestrator-1", { kind: "orchestrator", label: "Orchestrator", parentSessionId: null }),
      session("worker-a", { label: "Implementation worker", parentSessionId: "orchestrator-1" }),
    ];
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={sessions}
        runtimes={[runtime("worker-a")]}
        reasons={[]}
        events={[event(1, "worker-a", "editing MissionControl.tsx")]}
        activeSessionId="orchestrator-1"
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("Mission Control");
    expect(html).toContain("Orchestrator");
    expect(html).toContain("Implementation worker");
    expect(html).toContain("WORKING");
    expect(html).toContain("editing MissionControl.tsx");
  });

  it("surfaces a blocked worker as needing approval", () => {
    const sessions = [session("worker-b", { label: "Blocked worker", status: "waiting", parentSessionId: "orchestrator-1" })];
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={sessions}
        runtimes={[runtime("worker-b", { lifecycleState: "waiting" })]}
        reasons={[]}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("Needs your approval");
    expect(html).toContain("NEEDS YOU");
  });

  it("falls back to a reason when there are no streamed events", () => {
    const reasons: BridgeEvent[] = [{ id: 7, entityId: "worker-c", body: "waiting on write-scope approval", kind: "reason", source: "core", createdAt: "2026-07-29T10:04:00Z" }];
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={[session("worker-c", { label: "Reason worker" })]}
        runtimes={[runtime("worker-c")]}
        reasons={reasons}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("waiting on write-scope approval");
  });

  it("keeps a ready top-level session live instead of marking it DONE", () => {
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={[session("orchestrator-1", { kind: "orchestrator", label: "Ready orchestrator", status: "ready", parentSessionId: null })]}
        runtimes={[]}
        reasons={[]}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("Ready orchestrator");
    expect(html).toContain("READY");
    expect(html).not.toContain("DONE");
  });

  it("names missing runtime state instead of guessing success or hiding the worker", () => {
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={[session("worker-elsewhere", { label: "Foreign worker", status: "working", parentSessionId: "other-orchestrator" })]}
        runtimes={[]}
        reasons={[]}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("STATUS UNAVAILABLE");
    expect(html).toContain("Foreign worker");
    expect(html).not.toContain("Needs your approval");
  });

  it("drops finished sessions so completed history does not grow the grid", () => {
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={[session("worker-done", { label: "Finished worker", status: "completed" })]}
        runtimes={[runtime("worker-done", { lifecycleState: "completed" })]}
        reasons={[]}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("No agents running yet");
    expect(html).not.toContain("Finished worker");
  });

  it("does not promote unloaded historical workers into attention tiles", () => {
    const html = renderToStaticMarkup(<MissionControl sessions={[session("old", { parentSessionId: "old-parent", endedAt: "2026-09-01T00:00:00Z", status: "completed" })]} runtimes={[]} reasons={[]} events={[]} now={NOW} onFocusSession={() => undefined} />);
    expect(html).toContain("No agents running yet");
    expect(html).not.toContain("STATUS UNAVAILABLE");
  });

  it("shows an empty state when no agents are live", () => {
    const html = renderToStaticMarkup(
      <MissionControl sessions={[]} runtimes={[]} reasons={[]} events={[]} now={NOW} onFocusSession={() => undefined} />,
    );
    expect(html).toContain("No agents running yet");
  });

  it("hides idle sessions that the user is not currently focused on", () => {
    const html = renderToStaticMarkup(
      <MissionControl
        sessions={[session("idle-chat", { label: "Idle chat", status: "idle", kind: "direct", parentSessionId: null })]}
        runtimes={[]}
        reasons={[]}
        events={[]}
        now={NOW}
        onFocusSession={() => undefined}
      />,
    );
    expect(html).toContain("No agents running yet");
    expect(html).not.toContain("Idle chat");
  });
});
