// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BackgroundActivity, backgroundOutcome, describeBackgroundTask, formatElapsed, redirectTargets, type BackgroundOutcome } from "./BackgroundActivity";
import { reduceConversation, type ConversationItem } from "../conversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";

const NOW = Date.parse("2026-11-12T10:00:00Z");
const SHELL = "bun install >/tmp/bu-install.log 2>&1; (bun run build > /tmp/bu-build.log 2>&1; bun run test > /tmp/bu-test.log 2>&1)";

const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
  role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {},
  createdAt: "2026-11-12T09:55:46Z", ...overrides,
});

const task = (itemId: string, overrides: Partial<AgentEvent> = {}, data: Record<string, unknown> = {}): AgentEvent => event(1, "tool.started", {
  itemId, title: "Install, build and test", status: "inProgress", ...overrides,
  data: { type: "backgroundTask", backgroundTask: true, background: true, command: SHELL, ...data },
});

const item = (...events: AgentEvent[]): ConversationItem => {
  const items = reduceConversation(events);
  return items[items.length - 1];
};

describe("background task helpers", () => {
  it("reads the files a shell command writes into, without /dev/null or fd copies", () => {
    expect(redirectTargets(SHELL)).toEqual(["/tmp/bu-install.log", "/tmp/bu-build.log", "/tmp/bu-test.log"]);
    expect(redirectTargets("until grep -q done /tmp/x.log 2>/dev/null; do sleep 20; done")).toEqual([]);
    expect(redirectTargets("echo \"build exit $?\" >> /tmp/bu-build.log")).toEqual(["/tmp/bu-build.log"]);
    expect(redirectTargets(undefined)).toEqual([]);
  });

  it("formats elapsed time at the grain a person reads it", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(59_900)).toBe("59s");
    expect(formatElapsed(61_000)).toBe("1m 01s");
    expect(formatElapsed(3_725_000)).toBe("1h 02m");
  });

  it("names a task by what it is for, not by its shell", () => {
    const view = describeBackgroundTask(item(task("t")), NOW);
    expect(view.label).toBe("Install, build and test");
    expect(view.elapsedMs).toBe(254_000);
    expect(view.detail).toBe("Writing to bu-test.log");
    expect(view.logFiles).toEqual(["bu-install.log", "bu-build.log", "bu-test.log"]);
  });

  it("falls back past a provider's tool name to the command", () => {
    const view = describeBackgroundTask(item(task("t", { title: "Bash" }, { command: "cargo test --workspace" })), NOW);
    expect(view.label).toBe("cargo test --workspace");
  });

  it("prefers reported progress, then the newest output line, and drops news that repeats the label", () => {
    expect(describeBackgroundTask(item(task("t", {}, { progress: "Running bridge-core tests" })), NOW).detail).toBe("Running bridge-core tests");
    expect(describeBackgroundTask(item(task("t", { text: "one\ntwo\n" }, { aggregatedOutput: "one\ntwo\n" })), NOW).detail).toBe("two");
    expect(describeBackgroundTask(item(task("t", {}, { command: "sleep 60", progress: "Install, build and test" })), NOW).detail).toBeUndefined();
  });

  it("calls a passive watcher watching", () => {
    expect(describeBackgroundTask(item(task("w", {}, { passive: true })), NOW).kind).toBe("watching");
  });

  it("announces only outcomes the runtime reported", () => {
    const ended = (status: string, data: Record<string, unknown> = {}) => backgroundOutcome(item(task("t"), event(2, "tool.completed", { itemId: "t", status, data: { background: false, ...data } })));
    expect(ended("completed")).toBe("done");
    expect(ended("failed")).toBe("failed");
    expect(ended("completed", { exitCode: 1 })).toBe("failed");
    expect(ended("stopped")).toBe("stopped");
    expect(ended("unknown")).toBeUndefined();
    expect(ended("backgrounded")).toBeUndefined();
    expect(backgroundOutcome(item(task("t")))).toBeUndefined();
  });
});

describe("BackgroundActivity", () => {
  let host: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    MotionGlobalConfig.skipAnimations = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });
  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    MotionGlobalConfig.skipAnimations = false;
    vi.useRealTimers();
  });

  const render = (items: ConversationItem[], outcomeOf?: (key: string) => BackgroundOutcome | undefined, scope = "s") =>
    act(() => root.render(<BackgroundActivity items={items} harness="claude" scope={scope} outcomeOf={outcomeOf}/>));
  const pill = () => host.querySelector<HTMLButtonElement>("[data-background-pill]")!;
  const panel = () => host.querySelector("[data-background-panel]");

  it("draws nothing without background work", () => {
    render([]);
    expect(host.innerHTML).toBe("");
  });

  it("says the agent is working in the background, with one dot per task", () => {
    render([item(task("a")), item(task("b", {}, { passive: true }))]);
    expect(pill().textContent).toContain("Working in background");
    expect(pill().getAttribute("aria-label")).toBe("Working in background, 2 background tasks");
    expect(pill().getAttribute("aria-expanded")).toBe("false");
    expect(panel()).toBeNull();
  });

  it("says watching when every task is a watcher", () => {
    render([item(task("w", {}, { passive: true }))]);
    expect(pill().textContent).toContain("Watching in background");
  });

  it("opens into the list, opens a task's output, and closes on Escape", () => {
    render([item(task("a")), item(task("b", { title: "Rust tests", text: "test a ... ok\n" }, { command: "cargo test", aggregatedOutput: "test a ... ok\n" }))]);
    act(() => pill().click());
    expect(pill().getAttribute("aria-expanded")).toBe("true");
    const rows = [...panel()!.querySelectorAll<HTMLButtonElement>("li > button")];
    expect(rows.map(row => row.textContent)).toEqual([
      expect.stringContaining("Install, build and test"),
      expect.stringContaining("Rust tests"),
    ]);
    act(() => rows[0].click());
    expect(panel()!.textContent).toContain("Output goes to bu-install.log, bu-build.log, bu-test.log");
    act(() => rows[1].click());
    expect(panel()!.textContent).toContain("test a ... ok");
    expect(rows.map(row => row.getAttribute("aria-expanded"))).toEqual(["false", "true"]);
    act(() => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); });
    expect(pill().getAttribute("aria-expanded")).toBe("false");
  });

  it("names a finished task for a moment, then goes", () => {
    vi.useFakeTimers();
    const tests = item(task("t", { title: "Rust tests" }));
    render([tests]);
    render([], key => key === tests.identity || key === tests.key ? "done" : undefined);
    expect(host.querySelector("[data-background-notice]")?.textContent).toBe("Rust testsDone");
    expect(pill().disabled).toBe(true);
    act(() => { vi.advanceTimersByTime(4000); });
    expect(host.innerHTML).toBe("");
  });

  it("announces nothing when a task leaves without a reported outcome", () => {
    render([item(task("t"))]);
    render([], () => undefined);
    expect(host.innerHTML).toBe("");
  });

  it("never announces another chat's tasks", () => {
    render([item(task("t"))], undefined, "first");
    render([], () => "done", "second");
    expect(host.innerHTML).toBe("");
  });
});
