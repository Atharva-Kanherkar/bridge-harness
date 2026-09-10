// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { WorkBoard, WorkFact, WorkTask } from "../protocol/generated/protocol";
import { WorkView, type WorkViewProps } from "./WorkView";

const NOW = new Date("2026-09-10T12:00:00Z");
const task = (overrides: Partial<WorkTask> = {}): WorkTask => ({
  id: "slack-1", connectorInstanceId: "slack-work", sourceKind: "slack.message",
  title: "Release discussion", why: "The team shared an update.", rank: 1, confidenceBps: 8000,
  state: "active", pinned: false, missCount: 0, sourceActivityAt: "2026-09-10T11:00:00Z",
  createdAt: NOW.toISOString(), updatedAt: NOW.toISOString(),
  evidenceTarget: { kind: "externalLink", url: "https://app.slack.com/archives/C1/p1", host: "app.slack.com" }, ...overrides,
});
const board = (overrides: Partial<WorkBoard> = {}): WorkBoard => ({
  facts: [], tasks: [task()], sources: [], generatedAt: NOW.toISOString(),
  settings: { briefing: null, enabledConnectorInstances: [], refreshOnFocus: false, refreshIntervalMinutes: null, cooldownMinutes: 15, limits: { maxWallSeconds: 600, maxTurns: 12, maxToolCalls: 24 } },
  suggestions: { state: "ready" }, ...overrides,
});
let host: HTMLDivElement;
let root: Root;
function render(props: Partial<WorkViewProps> = {}) {
  act(() => root.render(<WorkView board={board()} now={NOW} onAction={async () => ({ ok: true })} onRefresh={() => {}} {...props} />));
}
const text = () => host.textContent ?? "";
const button = (label: string) => Array.from(host.querySelectorAll("button")).find(item => item.textContent?.includes(label))!;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

it("shows source activity and its date with an accessible integration list", () => {
  render();
  expect(host.querySelector('[aria-label="Integration activity"]')).not.toBeNull();
  expect(text()).toContain("Past 24 hours");
  expect(text()).toContain("Release discussion");
  expect(text()).toContain("Slack · slack-work");
  expect(host.querySelector("time")?.getAttribute("datetime")).toBe("2026-09-10T11:00:00Z");
});
it("never renders legacy local facts or their actions", () => {
  render({ board: board({ facts: [{ title: "cargo-test failed on Kyoto", kind: "failed_completion_check" } as WorkFact] }) });
  expect(text()).not.toContain("Kyoto");
  expect(text()).not.toMatch(/Needs you|confidence|Suggested|Start|Snooze|Pinned/);
});
it("offers only source navigation, never routes a summary into a local workspace", () => {
  const open = vi.fn(); const openTask = vi.fn();
  const item = task({ workspaceId: "ws-local" });
  render({ board: board({ tasks: [item] }), onOpenEvidence: open, onOpenTask: openTask });
  act(() => button("Open in app.slack.com").click());
  expect(open).toHaveBeenCalledWith(item); expect(openTask).not.toHaveBeenCalled();
});
it("does not offer unsafe or internal evidence targets", () => {
  render({ board: board({ tasks: [task({ evidenceTarget: { kind: "session", sessionId: "local" } })] }), onOpenEvidence: vi.fn() });
  expect(text()).not.toContain("Open in");
});
it("expires pinned, hidden, future and undated records instead of reviving cached work", () => {
  render({ board: board({ tasks: [task({ pinned: true, sourceActivityAt: "2026-09-08T12:00:00Z" }), task({ sourceActivityAt: null }), task({ state: "dismissed" }), task({ sourceActivityAt: "2026-09-11T12:00:00Z" })] }) });
  expect(text()).toContain("No recent activity to show"); expect(text()).not.toContain("Release discussion");
});
it("ages items out while the view stays open", () => {
  vi.useFakeTimers(); vi.setSystemTime(NOW);
  render({ now: undefined, board: board({ tasks: [task({ sourceActivityAt: "2026-09-09T12:00:10Z" })] }) });
  expect(text()).toContain("Release discussion");
  act(() => vi.advanceTimersByTime(30_000));
  expect(text()).not.toContain("Release discussion");
});
it("refresh runs the connected-tools briefing when configured", () => {
  const run = vi.fn(); const read = vi.fn(); render({ onRunBriefing: run, onRefresh: read });
  act(() => button("Refresh").click()); expect(run).toHaveBeenCalledOnce(); expect(read).not.toHaveBeenCalled();
});
it("disables refresh while a briefing is running", () => {
  render({ board: board({ suggestions: { state: "running" } }) }); expect(button("Refreshing").disabled).toBe(true);
});
it("offers setup with no invented activity when unconfigured", () => {
  const setup = vi.fn(); const run = vi.fn(); const read = vi.fn();
  render({ board: board({ tasks: [], suggestions: { state: "not_configured" } }), onOpenSettings: setup, onRunBriefing: run, onRefresh: read });
  act(() => button("Set up integrations").click()); expect(setup).toHaveBeenCalledOnce();
  act(() => button("Refresh").click()); expect(read).toHaveBeenCalledOnce(); expect(run).not.toHaveBeenCalled();
  expect(text()).toContain("No recent activity to show"); expect(text()).not.toContain("facts");
});
it("shows loading and retryable read errors", () => {
  const read = vi.fn(); render({ board: undefined }); expect(text()).toContain("Loading activity");
  render({ board: undefined, error: "Database unavailable", onRefresh: read });
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Database unavailable");
  act(() => button("Try again").click()); expect(read).toHaveBeenCalledOnce();
});
it("preserves only recent activity during partial refresh failures", () => {
  render({ refreshError: "Slack is disconnected", board: board({ tasks: [task(), task({ id: "old", title: "Old message", sourceActivityAt: "2026-09-08T12:00:00Z" })] }) });
  expect(text()).toContain("Slack is disconnected"); expect(text()).toContain("Release discussion"); expect(text()).not.toContain("Old message");
});
