// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ContextBreakdownPanel } from "./ContextBreakdown";
import type { ContextBreakdownResult, ContextBreakdownSegment } from "../protocol/generated/protocol";

function segment(overrides: Partial<ContextBreakdownSegment>): ContextBreakdownSegment {
  return { origin: "adapterInventory", segmentClass: "toolSchemas", state: "measured", capped: false, ...overrides };
}

function result(overrides: Partial<ContextBreakdownResult> = {}): ContextBreakdownResult {
  return {
    sessionId: "session-a",
    digest: "digest-a",
    segments: [
      segment({ segmentClass: "conversation", origin: "conversation", tokens: 58_400 }),
      segment({ segmentClass: "prompt-stable", origin: "promptCompilation", state: "estimated", tokens: 31_900, method: "token-estimate" }),
      segment({ segmentClass: "mcpDynamicTools", state: "reported", tokens: 8_900, itemCount: 29 }),
      segment({ segmentClass: "skillsPlugins", state: "unavailable", reason: "plugin payloads are not observable" }),
    ],
    totals: { tokens: 99_200, unavailableSources: 1 },
    conversation: { contextPressure: 0.78, contextWindowTokens: 200_000, entryCount: 12, renderedEntryCount: 12, tokenEstimate: 156_000 },
    compactionDelta: { boundaryEntryId: "e9", currentTokenEstimate: 156_000, firstRetainedEntryId: "e4", growthTokens: 18_240, reason: "post-checkpoint growth", sourceAgent: "orchestrator", tokensBefore: 110_000 },
    ...overrides,
  };
}

const idleState = { result: null as ContextBreakdownResult | null, reconciledAt: null, unavailable: false };

async function mountPanel(jsx: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(jsx));
  const html = () => container.innerHTML;
  const clickButtonWithText = async (text: string) => {
    const button = Array.from(container.querySelectorAll("button")).find(candidate => candidate.textContent?.includes(text));
    if (!button) throw new Error(`no button containing "${text}"`);
    await act(async () => button.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  };
  return {
    html,
    text: () => container.textContent ?? "",
    clickButtonWithText,
    unmount: () => act(async () => root.unmount()),
  };
}

describe("ContextBreakdownPanel", () => {
  it("renders all four wire source labels", async () => {
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    expect(handle.text()).toContain("Measured");
    expect(handle.text()).toContain("Estimated");
    expect(handle.text()).toContain("Reported");
    expect(handle.text()).toContain("Unavailable");
    await handle.unmount();
  });

  it("renders reasons for unavailable segments instead of zeros", async () => {
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    expect(handle.text()).toContain("plugin payloads are not observable");
    expect(handle.text()).not.toMatch(/Skills & plugins\s*0/);
    await handle.unmount();
  });

  it("ranks rows largest first with unavailable last", async () => {
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    const body = handle.text();
    const conversation = body.indexOf("Conversation");
    const promptStable = body.indexOf("Bridge sections · stable");
    const mcp = body.indexOf("MCP & dynamic tools");
    const skills = body.indexOf("Skills & plugins");
    expect(conversation).toBeLessThan(promptStable);
    expect(promptStable).toBeLessThan(mcp);
    expect(mcp).toBeLessThan(skills);
    await handle.unmount();
  });

  it("shows the compaction delta signed with its reason, and nothing without one", async () => {
    const withDelta = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    expect(withDelta.text()).toContain("+18,240 tok");
    expect(withDelta.text()).toContain("post-checkpoint growth · orchestrator");
    await withDelta.unmount();

    const stripped = result();
    delete (stripped as Partial<typeof stripped>).compactionDelta;
    const withoutDelta = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: stripped }} sessionId="session-a" onClose={() => undefined} />);
    expect(withoutDelta.text()).not.toContain("Since last checkpoint");
    await withoutDelta.unmount();
  });

  it("derives pressure copy from the shared contextPressure semantics", async () => {
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    expect(handle.text()).toContain("High pressure");
    expect(handle.text()).toContain("78%");
    await handle.unmount();
  });

  it("offers a Prompt Studio deep link on editable prompt segments", async () => {
    const onOpenPromptStudio = vi.fn();
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} onOpenPromptStudio={onOpenPromptStudio} />);
    await handle.clickButtonWithText("Edit in Prompt Studio");
    expect(onOpenPromptStudio).toHaveBeenCalledWith("prompt-stable");
    await handle.unmount();
  });

  it("states occupancy and headroom for the window", async () => {
    const handle = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, result: result() }} sessionId="session-a" onClose={() => undefined} />);
    expect(handle.text()).toContain("99,200 tok attributed");
    expect(handle.text()).toContain("44k free");
    await handle.unmount();
  });

  it("explains itself when no breakdown is available yet", async () => {
    const reconciling = await mountPanel(<ContextBreakdownPanel state={idleState} sessionId="session-a" onClose={() => undefined} />);
    expect(reconciling.text()).toContain("No breakdown recorded yet");
    await reconciling.unmount();

    const stopped = await mountPanel(<ContextBreakdownPanel state={{ ...idleState, unavailable: true }} sessionId="session-a" onClose={() => undefined} />);
    expect(stopped.text()).toContain("Polling stopped");
    await stopped.unmount();
  });
});
