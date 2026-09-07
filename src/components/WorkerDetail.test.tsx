import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { AgentEvent, Session, WorkerRuntimeRecord } from "../types";
import { asWireKind } from "../transcript/wire";
import { WorkerDetail } from "./WorkerDetail";

const NOW = Date.parse("2026-07-29T10:05:00Z");

const session: Session = {
  id: "worker-1",
  workspaceId: "workspace-1",
  harness: "codex",
  label: "implementation worker",
  status: "working",
  startedAt: "2026-07-29T10:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  kind: "worker",
  parentSessionId: "orchestrator-1",
};

const runtime: WorkerRuntimeRecord = {
  sessionId: "worker-1",
  parentSessionId: "orchestrator-1",
  lifecycleState: "working",
  taskFamily: "implementation",
  compatibilityKey: "implementation",
  resultStatus: "pending",
  retryCount: 1,
  warmUntil: null,
  worktreePath: null,
  worktreeBranch: null,
  lastResult: null,
  lastActivityAt: "2026-07-29T10:04:55Z",
  waitingSince: "2026-07-29T10:03:00Z",
  waitingReason: "approval_requested",
  progressSummary: "Running: cargo test",
  updatedAt: "2026-07-29T10:04:55Z",
};

const event = (id: number, kind: string, text: string, role: string | null = "assistant"): AgentEvent => ({
  id,
  sessionId: "worker-1",
  sequence: id,
  protocolVersion: 1,
  kind: asWireKind(kind),
  itemId: null,
  role,
  status: null,
  title: null,
  text,
  data: {},
  providerMeta: {},
  createdAt: "2026-07-29T10:04:00Z",
});

describe("WorkerDetail", () => {
  it("renders the activity feed, progress line, and waiting reason", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={session}
        runtime={runtime}
        liveEvents={[event(3, "message.completed", "Wrote the failing test first.")]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        initialEvents={[event(1, "tool.started", "cargo test"), event(2, "tool.completed", "cargo test — 12 passed")]}
      />,
    );
    expect(html).toContain("Running: cargo test");
    expect(html).toContain("waiting: approval requested");
    expect(html).toContain("Running cargo test");
    expect(html).toContain("Wrote the failing test first.");
    expect(html).toContain("retry 1");
  });

  it("deduplicates a live event already present in the durable backfill", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={session}
        runtime={{ ...runtime, waitingReason: null, waitingSince: null }}
        liveEvents={[event(1, "message.completed", "Only once, please.")]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        initialEvents={[event(1, "message.completed", "Only once, please.")]}
      />,
    );
    expect(html.split("Only once, please.").length - 1).toBe(1);
  });

  it("shows the result envelope once the worker has reported", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={{ ...session, status: "stopped" }}
        runtime={{ ...runtime, resultStatus: "reported", lastResult: { status: "completed", summary: "All acceptance criteria met." } }}
        liveEvents={[]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        initialEvents={[]}
      />,
    );
    expect(html).toContain("Result envelope");
    expect(html).toContain("All acceptance criteria met.");
  });

  it("offers a steering composer for a live worker", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={session}
        runtime={runtime}
        liveEvents={[]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        onSteer={async () => {}}
        initialEvents={[]}
      />,
    );
    expect(html).toContain("Steer this worker");
    expect(html).toContain("its orchestrator is told");
  });

  it("does not offer steering once the worker has reported", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={{ ...session, status: "stopped" }}
        runtime={{ ...runtime, resultStatus: "reported", lastResult: { status: "completed", summary: "Done." } }}
        liveEvents={[]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        onSteer={async () => {}}
        initialEvents={[]}
      />,
    );
    expect(html).not.toContain("Steer this worker");
    expect(html).toContain("Its typed result is final");
  });

  it("does not offer steering while the worker is checkpointing", () => {
    // Bridge's own checkpoint turn owns the provider; the backend refuses, so
    // the box must not be there to type into.
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={{ ...session, status: "checkpointing" }}
        runtime={{ ...runtime, lifecycleState: "checkpointing" }}
        liveEvents={[]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        onSteer={async () => {}}
        initialEvents={[]}
      />,
    );
    expect(html).not.toContain("Steer this worker");
  });

  it("shows no composer at all when the caller does not offer steering", () => {
    const html = renderToStaticMarkup(
      <WorkerDetail
        session={session}
        runtime={runtime}
        liveEvents={[]}
        now={NOW}
        onClose={() => {}}
        onFocusSession={() => {}}
        initialEvents={[]}
      />,
    );
    expect(html).not.toContain("Steer this worker");
    expect(html).not.toContain("Its typed result is final");
  });
});
