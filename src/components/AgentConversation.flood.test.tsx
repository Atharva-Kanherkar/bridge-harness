// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { FLOOD_STEPS, FLOOD_SUMMARY, floodStream } from "../transcript/fixtures/codexFlood";
import type { AgentEvent, Session } from "../types";

/**
 * A hundred-step turn, drawn.
 *
 * The reported defect: a turn that opens a fresh thought between every tool
 * call arrived as two hundred rows, every one of them expanded, and the pane
 * stopped responding. What a reader should get instead is five rows — their
 * message, the thought the model opened with, the run, the thought it closed
 * with, and the reply — with the hundred calls one click away.
 */

const session: Session = {
  id: "flood", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working",
  startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null,
  metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh",
  continuationFidelity: "native", kind: "orchestrator",
} as Session;

let host: HTMLDivElement;
let root: Root;

function mount(events: AgentEvent[]) {
  act(() => {
    root.render(<AgentConversation session={session} events={events} onResolve={() => {}} />);
  });
}

const rows = () => [...(host.querySelector(".max-w-3xl")?.children ?? [])];
const buttonWith = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(text));

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

describe("a hundred-step turn", () => {
  it("draws five top-level rows, not two hundred", () => {
    mount(floodStream());
    expect(rows()).toHaveLength(5);
  });

  it("says what the run did rather than only how long it was", () => {
    mount(floodStream());
    const summary = buttonWith(FLOOD_SUMMARY);
    expect(summary, `no row read "${FLOOD_SUMMARY}"`).toBeDefined();
    expect(summary?.textContent).toContain(`${FLOOD_STEPS} steps`);
  });

  it("keeps every call folded away until the reader asks", () => {
    mount(floodStream());
    // Not one command line, not one path, until the summary is clicked.
    expect(host.textContent).not.toContain("cargo test -p bridge-core");
    act(() => buttonWith(FLOOD_SUMMARY)!.click());
    expect(host.textContent).toContain("cargo test -p bridge-core");
  });

  it("keeps the thoughts that fell inside the run inside it, in order", () => {
    mount(floodStream());
    // The opening thought is a row of its own; the ninety-nine that followed a
    // call are in the group, and only visible once it is open.
    expect(host.textContent).toContain("Step 1: deciding what to do next.");
    expect(host.textContent).not.toContain("Step 50: deciding what to do next.");
    act(() => buttonWith(FLOOD_SUMMARY)!.click());
    const text = host.textContent ?? "";
    expect(text).toContain("Step 50: deciding what to do next.");
    expect(text.indexOf("Step 50:")).toBeLessThan(text.indexOf("Step 51:"));
  });

  it("holds the reader's choice when a later frame arrives", () => {
    const events = floodStream();
    mount(events.slice(0, -1));
    act(() => buttonWith(FLOOD_SUMMARY)!.click());
    expect(host.textContent).toContain("cargo test -p bridge-core");
    // A flush that changes an unrelated part of the turn must not slam the
    // group the reader just opened.
    mount(events);
    expect(host.textContent).toContain("cargo test -p bridge-core");
  });

  it("bounds the rows however many more steps arrive", () => {
    mount(floodStream(400));
    expect(rows()).toHaveLength(5);
  });
});
