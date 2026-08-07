import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { Session, WorkerRuntimeRecord } from "../types";
import { SidebarWorkerPanel } from "./SidebarWorkerPanel";

const worker: Session = {
  id: "worker-1",
  workspaceId: "workspace-1",
  harness: "codex",
  label: "Planning worker",
  status: "working",
  startedAt: "2026-07-29T10:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  parentSessionId: "parent-1",
  restorationMode: "fresh", continuationFidelity: "native", kind: "worker",
};

const runtime: WorkerRuntimeRecord = {
  sessionId: worker.id,
  parentSessionId: "parent-1",
  lifecycleState: "working",
  taskFamily: "planning",
  compatibilityKey: "planning",
  resultStatus: "pending",
  retryCount: 1,
  warmUntil: null,
  worktreePath: null,
  worktreeBranch: null,
  lastResult: null,
  lastActivityAt: "2026-07-29T10:00:50Z",
  updatedAt: "2026-07-29T10:00:50Z",
};

describe("SidebarWorkerPanel", () => {
  it("shows live lifecycle, role, retry count, and latest activity", () => {
    const html = renderToStaticMarkup(
      <SidebarWorkerPanel
        workers={[worker]}
        runtimes={[runtime]}
        reasons={[{ id: 1, source: "worker", kind: "worker.activity", entityId: worker.id, body: "Inspecting the router schema", createdAt: runtime.updatedAt }]}
        collapsed={false}
        now={Date.parse("2026-07-29T10:01:00Z")}
      />,
    );
    expect(html).toContain("Live workers");
    expect(html).toContain("Planning worker");
    expect(html).toContain("WORKING");
    expect(html).toContain("planning");
    expect(html).toContain("retry 1");
    expect(html).toContain("Inspecting the router schema");
    expect(html).toContain("10s ago");
  });

  it("keeps a live worker indicator in the collapsed sidebar", () => {
    const html = renderToStaticMarkup(<SidebarWorkerPanel workers={[worker]} runtimes={[runtime]} reasons={[]} collapsed now={0} />);
    expect(html).toContain("1 worker: 1 running");
    expect(html).toContain("animate-pulse");
  });

  it("shows failures and waiting workers even while another worker is active", () => {
    const failed = { ...worker, id: "worker-2", label: "Failed worker", status: "failed" as const };
    const waiting = { ...worker, id: "worker-3", label: "Waiting worker", status: "waiting" as const };
    const html = renderToStaticMarkup(
      <SidebarWorkerPanel workers={[worker, failed, waiting]} runtimes={[runtime]} reasons={[]} collapsed={false} now={0} />,
    );
    expect(html).toContain("1 active");
    expect(html).toContain("1 waiting");
    expect(html).toContain("1 failed");
  });
});
