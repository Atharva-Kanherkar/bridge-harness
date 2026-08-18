import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { MissionControl } from "./MissionControl";

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
  updatedAt: "2026-07-29T10:04:55Z",
  ...overrides,
});

const event = (id: number, sessionId: string, text: string): AgentEvent => ({
  id,
  sessionId,
  sequence: id,
  protocolVersion: 1,
  kind: "reasoning",
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
