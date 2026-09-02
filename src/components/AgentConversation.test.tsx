// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { AgentEvent, CompletionSummary, Session, SessionEntry, WorkerRuntimeRecord } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" };
const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides });
const completion = (verdict: CompletionSummary["verdict"]): CompletionSummary => ({ attemptId:"a",contractId:"c",verdict,repository:{head:"abcdef1234567890",dirtyDigest:"clean"},passedRequired:0,totalRequired:1,markdownCommitted:false,waiverReason:verdict === "waived" ? "Accepted risk" : null,checks:[{checkId:"gate",kind:"deterministic",required:true,status:verdict === "verified" ? "passed" : verdict === "changes_requested" ? "failed" : verdict === "superseded" ? "stale" : verdict === "waived" ? "skipped" : "pending",executor:"bridge.shell",command:"bun test",verifierFamily:null,detail:null,outputDigest:verdict === "verified" ? "digest" : null,artifactRefs:[]}] });

describe("AgentConversation", () => {
  const editToolEvent = event(1, "tool.started", { itemId: "t", title: "Edit src/App.tsx", status: "completed", data: { type: "fileChange", path: "src/App.tsx" } });

  async function mountConversation(extraProps: Record<string, unknown>) {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<AgentConversation session={session} onResolve={() => undefined} events={[editToolEvent]} {...extraProps} />));
    const group = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Edited"));
    if (group) await act(async () => {
      group.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    return { container, unmount: async () => { await act(async () => root.unmount()); container.remove(); } };
  }

  it("makes an edit tool's path a link into the Code pane when an opener exists", async () => {
    const onOpenFile = vi.fn();
    const { container, unmount } = await mountConversation({ workspaceFiles: ["src/App.tsx"], onOpenFile });
    const link = container.querySelector<HTMLButtonElement>('button[aria-label="Open src/App.tsx in the Code pane"]')!;
    expect(link).not.toBeNull();
    await act(async () => {
      link.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onOpenFile).toHaveBeenCalledWith("src/App.tsx", undefined);
    await unmount();
  });

  it("leaves the tool path inert without an opener", async () => {
    const { container, unmount } = await mountConversation({});
    expect(container.textContent).toContain("src/App.tsx");
    expect(container.querySelector('button[aria-label="Open src/App.tsx in the Code pane"]')).toBeNull();
    await unmount();
  });


  // A session left on its adapter's default stores no model id. `modelLabel`
  // renders that absence as an em dash, which the narration row would have
  // read out as "— is reading your message…".
  it("names the harness in the startup row when the session carries no model id", () => {
    const html = renderToStaticMarkup(<AgentConversation session={{ ...session, model: null }} onResolve={() => undefined} events={[]} working />);
    expect(html).toContain("Codex is reading your message");
    expect(html).not.toContain("— is reading your message");
  });

  it("names the model in the startup row when the session has one", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} working />);
    expect(html).toContain("GPT Luna is reading your message");
  });

  // The startup row's whole job is to say *which* agent is starting and how
  // long it has been. Contract: testing/feat-startup-mark-and-switch-checkpoint.md.
  it("wears the harness's own mark while starting, turning", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} working />);
    expect(html).toContain("text-harness-codex");
    expect(html).toContain("harness-mark-live");
    expect(html).not.toContain("text-harness-claude");
  });

  it("marks a Claude session with Claude's figure, not Codex's", () => {
    const html = renderToStaticMarkup(<AgentConversation session={{ ...session, harness: "claude" }} onResolve={() => undefined} events={[]} working />);
    expect(html).toContain("text-harness-claude");
    expect(html).not.toContain("text-harness-codex");
  });

  it("keeps the mark and drops the label once streaming has begun", () => {
    const streamingEvent = event(1, "message.completed", { itemId: "a", role: "assistant", status: "streaming", text: "Wor" });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[streamingEvent]} working />);
    expect(html).not.toContain("is reading your message");
  });

  // Elapsed reads before the label, the way the reference CLI does it, and in
  // tabular figures so a second ticking over cannot reflow the words beside it.
  it("puts the elapsed counter ahead of the label once past 2s", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    try {
      (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      await act(async () => root.render(<AgentConversation session={session} onResolve={() => undefined} events={[]} working />));
      await act(async () => { vi.advanceTimersByTime(2400); });
      const line = container.querySelector<HTMLElement>(".tabular-nums")!;
      expect(line).not.toBeNull();
      expect(line.textContent).toContain("2s");
      const row = line.parentElement!;
      expect(row.textContent).toMatch(/^2s · GPT Luna is reading your message/);
      await act(async () => root.unmount());
      container.remove();
    } finally {
      vi.useRealTimers();
    }
  });

  // The card used to print the reason twice: once as body text and once as a raw
  // <code> block, which is how a model switch read `before_downgrade` twice.
  it("states a compaction reason once, in English", () => {
    const requested: SessionEntry = { id: "e1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2, kind: "compaction.requested", payload: { reason: "before_downgrade", attempt: 0 }, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={[requested]} activeLeafId="e1" />);
    expect(html).toContain("Before switching models");
    expect(html).not.toContain("before_downgrade");
    expect(html.split("Before switching models").length - 1).toBe(1);
  });

  it("explains compaction recovery and prevents duplicate retries", async () => {
    let rejectRetry!: (reason: Error) => void;
    const onRetryCompaction = vi.fn(() => new Promise<void>((_resolve, reject) => { rejectRetry = reject; }));
    const failed: SessionEntry = {
      id: "failed",
      sessionId: "s",
      parentEntryId: null,
      sequence: 1,
      semanticSchemaVersion: 2,
      kind: "compaction.failed",
      payload: {
        reason: "checkpoint turn could not start: provider pipe is closed",
        message: "Compaction could not reach a ready provider. The original conversation history is intact.",
        retryable: true,
        recoveryAction: "Retry compaction when the provider is ready, or keep working with the original history.",
      },
      providerEventId: null,
      contextVisibility: "eligible",
      tokenEstimate: null,
      createdAt: "now",
    };
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={[failed]} activeLeafId="failed" onRetryCompaction={onRetryCompaction}/>));

    expect(container.textContent).toContain("original conversation history is intact");
    expect(container.textContent).toContain("Retry compaction when the provider is ready");
    expect(container.textContent).not.toContain("provider pipe is closed");
    const retry = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Retry compaction"))!;
    await act(async () => {
      retry.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      retry.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onRetryCompaction).toHaveBeenCalledTimes(1);
    expect(retry.disabled).toBe(true);

    await act(async () => rejectRetry(new Error("The provider is still offline")));
    expect(container.textContent).toContain("The provider is still offline");
    expect(retry.disabled).toBe(false);
    await act(async () => root.unmount());
    container.remove();
  });

  // Lifecycle plumbing replayed as collapsed "Used tools" groups before and
  // around the user's messages after a reload; a model change replayed as a
  // group of one. Contract: fix-cross-harness-switch-and-shell-polish.md §3.
  it("keeps lifecycle entries out of a replayed transcript", () => {
    const machine = (id: string, kind: string, sequence: number, payload: Record<string, unknown> = {}): SessionEntry =>
      ({ id, sessionId: "s", parentEntryId: sequence === 1 ? null : `e${sequence - 1}`, sequence, semanticSchemaVersion: 2, kind, payload, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" });
    const entries = [
      machine("e1", "session.started", 1, { status: "ready" }),
      machine("e2", "user.message", 2, { text: "hi" }),
      machine("e3", "session.status", 3, { status: "working" }),
      machine("e4", "assistant.message", 4, { role: "assistant", text: "Hey! What are we building?" }),
      machine("e5", "turn.completed", 5, {}),
    ];
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={entries} activeLeafId="e5" />);
    expect(html).toContain("What are we building?");
    expect(html).not.toContain("Used tools");
    expect(html).not.toContain("used tools");
  });

  it("renders a replayed model change as a divider, not a tools group", () => {
    const changed: SessionEntry = { id: "e1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2, kind: "session.model_changed", payload: { role: "system", status: "ready", title: "Chat model changed", text: "Chat runtime changed.", data: { previousHarness: "codex", previousModel: "gpt-5.6-luna", harness: "claude", model: "opus", freshProviderSession: true } }, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={[changed]} activeLeafId="e1" />);
    expect(html).toContain("Codex · GPT Luna → Claude · Opus");
    expect(html).not.toContain("Used tools");
  });

  it("narrates a model switch with the incoming harness's mark, and no first-launch note anywhere", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} modelSwitch={{ harness: "claude", label: "Opus" }} />);
    expect(html).toContain("Switching to Opus…");
    expect(html).toContain("text-harness-claude");
    expect(html).not.toContain("text-harness-codex");
    expect(html).not.toContain("First time opening this chat");
  });

  it("shows revision-bound verification without requiring a committed contract file", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} completion={{ attemptId:"a",contractId:"c",verdict:"waived",repository:{head:"abcdef1234567890",dirtyDigest:"clean"},passedRequired:1,totalRequired:2,markdownCommitted:false,waiverReason:"Browser unavailable",checks:[{checkId:"tests",kind:"deterministic",required:true,status:"passed",executor:"bridge.shell",command:"bun test",verifierFamily:null,detail:"159 passed",outputDigest:"d",artifactRefs:[]},{checkId:"journey",kind:"user_testing",required:true,status:"skipped",executor:"bridge.worker",command:null,verifierFamily:"claude",detail:"No browser",outputDigest:null,artifactRefs:[]}]} } />);
    expect(html).toContain("Verified with waiver");
    expect(html).toContain("private contract");
    expect(html).toContain("abcdef123456");
    expect(html).toContain("Browser unavailable");
    expect(html).toContain("Skipped");
  });

  it.each([
    ["verifying", "Verifying", "Pending"],
    ["changes_requested", "Changes requested", "Failed"],
    ["verified", "Verified", "Passed"],
    ["superseded", "Evidence superseded", "Stale"],
    ["waived", "Verified with waiver", "Skipped"],
  ] as const)("renders %s as a distinct proof state", (verdict, title, checkStatus) => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} completion={completion(verdict)}/>);
    expect(html).toContain(title);
    expect(html).toContain(checkStatus);
  });

  it("offers a human waiver only for unresolved nonterminal proof", () => {
    const open = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} onWaiveCompletion={async () => undefined} events={[]} completion={completion("changes_requested")}/>);
    expect(open).toContain("Waive unresolved checks");
    const verified = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} onWaiveCompletion={async () => undefined} events={[]} completion={completion("verified")}/>);
    expect(verified).not.toContain("Waive unresolved checks");
  });

  it("renders normalized primitives as GUI cards without a terminal surface", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "message.completed", { itemId: "m", role: "assistant", text: "Structured response", status: "completed" }),
      event(2, "plan.updated", { title: "Plan", data: { plan: [{ step: "Render GUI", status: "completed" }] } }),
      event(3, "approval.requested", { title: "Approve command", status: "pending", data: { command: "bun test" } })
    ]}/>);
    expect(html).toContain("Structured response");
    expect(html).toContain("Approve command");
    expect(html).not.toContain("terminal-host");
    expect(html).not.toContain("xterm");
  });
  const staleBase = (divergence: Record<string, unknown>) => [
    event(1, "workspace.stale_base", {
      title: "Workspace is behind",
      text: "this workspace is behind origin/main",
      data: { staleBase: true, divergence },
    }),
  ];

  it("does not offer a refresh a fast-forward cannot perform", () => {
    // The reported case: a fresh worktree with one commit on it. `refresh` is a
    // strict fast-forward, so the button could only ever produce an error.
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined}
      events={staleBase({ behind: 104, ahead: 1, baseRef: "origin/main", dirty: false })}
      onRefreshBase={async () => undefined}/>);
    expect(html).not.toContain("Refresh workspace");
    expect(html).toContain("cannot be fast-forwarded");
    expect(html).toContain("1 commit that origin/main does not");
  });

  it("does not offer a refresh over uncommitted changes", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined}
      events={staleBase({ behind: 104, ahead: 0, baseRef: "origin/main", dirty: true })}
      onRefreshBase={async () => undefined}/>);
    expect(html).not.toContain("Refresh workspace");
    expect(html).toContain("uncommitted changes");
  });

  it("still offers the refresh when a fast-forward is possible", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined}
      events={staleBase({ behind: 104, ahead: 0, baseRef: "origin/main", dirty: false })}
      onRefreshBase={async () => undefined}/>);
    expect(html).toContain("Refresh workspace");
    expect(html).not.toContain("cannot be fast-forwarded");
  });

  it("renders active forest cards and hides raw provider frames", () => {
    const entry = (id:string,parentEntryId:string|null,kind:string,payload:Record<string,unknown>,sequence:number,contextVisibility="eligible"):SessionEntry => ({ id,sessionId:"s",parentEntryId,sequence,semanticSchemaVersion:2,kind,payload,providerEventId:null,contextVisibility,tokenEstimate:null,createdAt:"now" });
    const entries = [entry("one",null,"checkpoint",{summary:"Durable checkpoint"},1),entry("two","one","compaction",{summary:"Reduced context",reason:"manual"},2),entry("three","two","provider.unknown",{title:"Provider frame",raw:"secret"},3,"raw")];
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={entries} activeLeafId="three"/>);
    expect(html).toContain("Durable checkpoint");
    expect(html).toContain("Reduced context");
    // Raw provider telemetry is internal, not conversation — it must not render.
    expect(html).not.toContain("Provider frame");
    expect(html).not.toContain("secret");
  });
  it("offers only same-turn approval for delegation path scope", () => {
    const entry: SessionEntry = { id:"approval",sessionId:"s",parentEntryId:null,sequence:4,semanticSchemaVersion:2,kind:"approval.requested",payload:{status:"pending",approvalType:"delegation_path_scope",title:"Approve delegation write scope"},providerEventId:null,contextVisibility:"eligible",tokenEstimate:null,createdAt:"now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={[entry]} activeLeafId="approval"/>);
    expect(html).toContain("Allow once");
    expect(html).not.toContain("Allow for session");
  });
  it("renders only provider-offered permission actions and makes policy decisions inert", () => {
    const requested = event(41, "permission.requested", { status:"pending", title:"Run command", data:{actions:[{decision:"accept",optionId:"yes-once",label:"Proceed once"}]} });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[requested]}/>);
    expect(html).toContain("Proceed once");
    expect(html).not.toContain("Allow for session");
    expect(html).not.toContain("Decline");

    const automatic = event(42, "permission.requested", { status:"settling", title:"Run command", data:{resolvedBy:"policy",reason:"Auto-approve provider permissions",actions:[{decision:"accept",label:"Allow once"}]} });
    const policyHtml = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[automatic]}/>);
    expect(policyHtml).toContain("Applying decision");
    expect(policyHtml).toContain("by policy");
    expect(policyHtml).not.toContain("Allow once</button>");
  });

  it("holds permission actions disabled until a deferred resolution settles", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    let release!: () => void;
    const deferred = new Promise<void>(resolve => { release = resolve; });
    const onResolve = vi.fn(() => deferred);
    const requested = event(43, "permission.requested", { status:"pending", title:"Run command", data:{actions:[{decision:"accept",optionId:"once",label:"Allow once"},{decision:"decline",optionId:"no",label:"Decline"}]} });
    await act(async () => root.render(<AgentConversation session={session} onResolve={onResolve} events={[requested]}/>));
    const allow = [...container.querySelectorAll("button")].find(button => button.textContent === "Allow once")!;
    await act(async () => allow.dispatchEvent(new MouseEvent("click", { bubbles:true })));
    expect(onResolve).toHaveBeenCalledWith(43, "accept", "once");
    expect([...container.querySelectorAll("button")].every(button => button.disabled)).toBe(true);
    expect(container.textContent).toContain("Applying…");
    await act(async () => release());
    await act(async () => root.unmount());
    container.remove();
  });
  it("surfaces a rejected delegation as a distinct row instead of silently dropping it", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "delegation.rejected", { role: "system", status: "failed", title: "Delegation rejected", text: "unknown variant `none`", data: { reason: "unknown variant `none`", willRetry: true, attempt: 1 } })
    ]}/>);
    expect(html).toContain("Delegation rejected");
    expect(html).toContain("no worker started");
    expect(html).toContain("unknown variant");
    expect(html).toContain("correct and re-emit");
    expect(html).toContain('role="alert"');
  });
  it("shows launch failures and confirms that the orchestrator will not wait", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "delegation.rejected", { role: "system", status: "failed", title: "Worker failed to start", text: "router schema mismatch", data: { launchFailed: true, phase: "routing", reason: "router schema mismatch", willRetry: false, orchestratorNotified: true } })
    ]}/>);
    expect(html).toContain("Worker failed to start");
    expect(html).toContain("router schema mismatch");
    expect(html).toContain("orchestrator was notified");
    expect(html).toContain("will not wait");
  });
  it("offers adopt and discard for changes that never reached the workspace", () => {
    const binding = { sessionId: "child", parentSessionId: "s", workspaceId: "w", worktreePath: "/tmp/workers/child", worktreeBranch: "bridge/task-worker-child", taskWorktreePath: "/tmp/task", state: "pending_adoption", head: "2b43aaad", baseCommit: "90ce51c", baseBranch: "bridge/task", baselineDirtyPaths: [], changedPaths: ["src/components/Markdown.tsx"], diffstat: "1 file(s) changed, 12 insertion(s), 3 deletion(s)", dirty: false, detail: null, createdAt: "now", updatedAt: "now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} pendingAdoptions={[binding]} onResolveAdoption={async () => undefined}/>);
    expect(html).toContain("not in your workspace yet");
    expect(html).toContain("src/components/Markdown.tsx");
    expect(html).toContain("1 file(s) changed");
    expect(html).toContain("bridge/task-worker-child");
    expect(html).toContain("Adopt changes");
    expect(html).toContain("Discard");
    expect(html).toContain("stays unfinished");
    expect(html).toContain('role="alert"');
  });
  it("disables the adoption choice while a decision is already settling", () => {
    const binding = { sessionId: "child", parentSessionId: "s", workspaceId: "w", worktreePath: "/tmp/workers/child", worktreeBranch: "b", taskWorktreePath: "/tmp/task", state: "settling", head: null, baseCommit: null, baseBranch: null, baselineDirtyPaths: [], changedPaths: [], diffstat: null, dirty: true, detail: "adopt in progress", createdAt: "now", updatedAt: "now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} pendingAdoptions={[binding]} onResolveAdoption={async () => undefined}/>);
    expect(html).toContain("settling");
    expect(html).toContain("disabled");
  });
  it("shows the routing reason, remediation, and write scope on a delegation approval", () => {
    const entry: SessionEntry = { id:"approval",sessionId:"s",parentEntryId:null,sequence:5,semanticSchemaVersion:2,kind:"approval.requested",payload:{status:"pending",approvalType:"delegation_path_scope",title:"Approve delegation write scope",objective:"Render Mermaid inline",reason:"owned_path_provenance_required",remediation:"these write paths were proposed by the agent and were not explicitly authorized.",requestedOwnedPaths:["src/**","docs/**"]},providerEventId:null,contextVisibility:"eligible",tokenEstimate:null,createdAt:"now" };
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} forestEntries={[entry]} activeLeafId="approval"/>);
    expect(html).toContain("Write scope");
    expect(html).toContain("src/**");
    expect(html).toContain("docs/**");
    expect(html).toContain("owned_path_provenance_required");
    expect(html).toContain("were not explicitly authorized");
    expect(html).toContain("Render Mermaid inline");
  });
  it("mirrors a background worker's approval onto the parent instead of calling it a failure", () => {
    const blocked = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "delegation.blocked", { role: "system", status: "waiting", title: "Implementation · strong needs your approval", text: "Run bun install?", data: { childBlocked: true, childSessionId: "child", label: "Implementation · strong", objective: "Render Mermaid inline", command: "bun install", cwd: "/repo", ownedPaths: ["src/**"], orchestratorNotified: true } })
    ]}/>);
    expect(blocked).toContain("needs your approval");
    expect(blocked).toContain("bun install");
    expect(blocked).toContain("/repo");
    expect(blocked).toContain("write scope: src/**");
    expect(blocked).toContain("idle until you do");
    expect(blocked).toContain('role="alert"');
    // A blocked child must never be presented as a failed launch.
    expect(blocked).not.toContain("Worker failed to start");
    expect(blocked).not.toContain("no worker started");

    const resolved = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "delegation.blocked", { role: "system", status: "accept", title: "Implementation · strong approval accept", data: { childBlocked: false, childSessionId: "child", label: "Implementation · strong", outcome: "accept" } })
    ]}/>);
    expect(resolved).toContain("approval accept");
    expect(resolved).not.toContain('role="alert"');
  });
  it("gives the mirrored block a way to reach the worker's own approval", () => {
    const data = { childBlocked: true, childSessionId: "child-77", label: "Implementation · strong", command: "bun install", ownedPaths: ["src/**"] };
    const actionable = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} onOpenSession={() => undefined} events={[
      event(1, "delegation.blocked", { role: "system", status: "waiting", title: "worker needs your approval", data })
    ]}/>);
    expect(actionable).toContain("Open worker to approve");
    // Without a navigation handler the block stays a plain instruction, never a dead button.
    const inert = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "delegation.blocked", { role: "system", status: "waiting", title: "worker needs your approval", data })
    ]}/>);
    expect(inert).not.toContain("Open worker to approve");
    expect(inert).toContain("Open the worker");
  });
  it("surfaces conversation and file-state divergence", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} repositoryDivergence="diverged"/>);
    expect(html).toContain("This branch&#x27;s context predates the current file state.");
    expect(html).toContain('role="alert"');
  });
  it("names a worker's real failure cause and offers the retry Bridge stopped taking", () => {
    const failed = event(20, "delegation.result", {
      itemId: "result-w1", role: "system", status: "completed", title: "Worker result",
      text: "Could not finish the migration guard.",
      data: {
        childSessionId: "w1", delivered: true, status: "failed",
        failureClass: "permanent",
        failureCause: "a check the worker ran failed: cargo test store",
        canRetry: true,
      },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[failed]} onRetryWorker={async () => undefined} onOpenSession={() => undefined}/>);
    // The cause, not "Subagent finished".
    expect(html).toContain("a check the worker ran failed: cargo test store");
    expect(html).toContain("Retry this task");
    expect(html).toContain('role="alert"');
    expect(html).not.toContain("Subagent finished");
  });

  it("says plainly when a worker's result could not be read, and does not dress it as a task failure", () => {
    const unreadable = event(21, "delegation.result", {
      itemId: "result-w2", role: "system", status: "completed", title: "Worker result",
      text: "I refactored the store and the tests pass.",
      data: {
        childSessionId: "w2", delivered: true, status: "protocol_invalid",
        failureClass: "protocol_invalid",
        failureCause: "the worker's result could not be read as a result",
        canRetry: true,
      },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[unreadable]} onRetryWorker={async () => undefined}/>);
    expect(html).toContain("could not be read");
    expect(html).toContain("nothing below has been verified");
    // The worker's own words survive, unverified.
    expect(html).toContain("the tests pass");
  });

  it("leaves a successful worker result as the quiet row it was", () => {
    const done = event(22, "delegation.result", {
      itemId: "result-w3", role: "system", status: "completed", title: "Worker result",
      text: "Auth module refactored.",
      data: { childSessionId: "w3", delivered: true, status: "completed", canRetry: false },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[done]} onRetryWorker={async () => undefined}/>);
    expect(html).toContain("Subagent finished");
    expect(html).not.toContain("Retry this task");
  });

  /* ── The live worker panel ──────────────────────────────────────────── */

  const workerSession = (overrides: Partial<Session> = {}): Session => ({
    ...session, id: "w1", label: "Implementation · strong", parentSessionId: "s", depth: 1,
    kind: "workspace", startedAt: "2026-08-21T10:00:00Z", ...overrides,
  });
  const workerRuntime = (overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
    sessionId: "w1", parentSessionId: "s", lifecycleState: "working", taskFamily: "implementation",
    compatibilityKey: "key", resultStatus: "pending", retryCount: 0, warmUntil: null, worktreePath: null,
    worktreeBranch: null, lastResult: null, lastActivityAt: null, waitingSince: null, waitingReason: null,
    progressSummary: null, updatedAt: "now", ...overrides,
  });
  const spawned = event(30, "delegation.spawned", {
    itemId: "spawn-w1", role: "system", status: "working", title: "Delegated to Implementation · strong",
    text: "Add refresh-token rotation",
    data: { childSessionId: "w1", modelLabel: "Fable", effort: "high" },
  });
  const workerActivity = (id: number, title: string): AgentEvent => event(id, "tool.started", { sessionId: "w1", title });

  it("shows a live worker panel while the worker runs", () => {
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned]}
      now={Date.parse("2026-08-21T10:02:30Z")}
      workers={{
        sessions: [session, workerSession()],
        runtimes: [workerRuntime({ retryCount: 1, progressSummary: "editing src/auth/store.rs" })],
        events: [workerActivity(31, "read store.rs"), workerActivity(32, "edit store.rs")],
      }}
      onOpenSession={() => undefined}
      onExpandWorker={() => undefined}
    />);
    expect(html).toContain("Implementation · strong");
    expect(html).toContain("WORKING");
    expect(html).toContain("editing src/auth/store.rs");
    expect(html).toContain("retry 1");
    // The mini-feed is the whole point: something visibly moving in the chat.
    expect(html).toContain("edit store.rs");
    expect(html).toContain("Expand");
    expect(html).toContain("Open session");
    // And the old static line is gone.
    expect(html).not.toContain("Delegated · Delegated to");
  });

  it("names the waiting reason instead of showing a stalled panel", () => {
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned]}
      now={Date.parse("2026-08-21T10:02:30Z")}
      workers={{
        sessions: [session, workerSession({ status: "waiting" })],
        runtimes: [workerRuntime({ lifecycleState: "waiting", waitingReason: "approval_requested" })],
        events: [],
      }}
    />);
    expect(html).toContain("NEEDS YOU");
    expect(html).toContain("waiting: approval requested");
  });

  it("turns the same panel into the result card when the result lands", () => {
    const result = event(33, "delegation.result", {
      itemId: "result-w1", role: "system", status: "completed", title: "Worker result",
      text: "Rotation added.", data: { childSessionId: "w1", delivered: true, status: "completed" },
    });
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned, result]}
      now={Date.parse("2026-08-21T10:05:00Z")}
      workers={{
        sessions: [session, workerSession({ status: "stopped" })],
        runtimes: [workerRuntime({
          resultStatus: "reported", lifecycleState: "completed",
          lastResult: { status: "completed", summary: "Rotation added.", filesChanged: ["src/auth/store.rs"], tests: [{ command: "cargo test auth", status: "passed" }] },
        })],
        events: [workerActivity(31, "edit store.rs")],
      }}
    />);
    expect(html).toContain("DONE");
    expect(html).toContain("1 file");
    expect(html).toContain("1 test passing");
    // One card, not a live panel plus a disconnected outcome row.
    expect(html).not.toContain("Subagent finished");
    // And the live ticker stops: no half-finished feed under a finished result.
    expect(html).not.toContain("edit store.rs");
  });

  it("still shows a classified failure with its retry action after folding", () => {
    const failed = event(34, "delegation.result", {
      itemId: "result-w1", role: "system", status: "failed", title: "Worker finished without completing",
      text: "Nothing was changed.",
      data: { childSessionId: "w1", delivered: true, status: "failed", failureCause: "the worker stopped responding", failureClass: "stalled", canRetry: true },
    });
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned, failed]}
      workers={{ sessions: [session, workerSession()], runtimes: [workerRuntime()], events: [] }}
      onRetryWorker={async () => undefined}
      onOpenSession={() => undefined}
    />);
    expect(html).toContain("the worker stopped responding");
    expect(html).toContain("Retry this task");
  });

  it("falls back to the quiet row when the worker's session is not loaded yet", () => {
    // The spawn event can beat the state poll that carries the child session row.
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned]}
      workers={{ sessions: [session], runtimes: [], events: [] }}
    />);
    expect(html).toContain("Delegated");
    expect(html).not.toContain("Expand");
  });

  it("shows a chip when someone steers a worker", () => {
    const steered = event(35, "delegation.steered", {
      itemId: "steer-1", role: "system", status: "delivered", title: "You steered Implementation · strong",
      text: "use the existing store",
      data: { childSessionId: "w1", label: "Implementation · strong", steeredBy: "user", steerDelivered: true, landed: "now", orchestratorNotified: true },
    });
    const html = renderToStaticMarkup(<AgentConversation
      session={session}
      onResolve={() => undefined}
      events={[spawned, steered]}
      workers={{ sessions: [session, workerSession()], runtimes: [workerRuntime()], events: [] }}
      onOpenSession={() => undefined}
    />);
    expect(html).toContain("You steered Implementation · strong");
    expect(html).toContain("use the existing store");
    expect(html).not.toContain("NOT DELIVERED");
  });

  it("says so when a steer never reached the worker", () => {
    const undelivered = event(36, "delegation.steered", {
      itemId: "steer-2", role: "system", status: "undelivered", title: "Orchestrator steered Implementation · strong",
      data: { childSessionId: "w1", label: "Implementation · strong", steeredBy: "orchestrator", steerDelivered: false, landed: "undelivered", orchestratorNotified: true },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[undelivered]}/>);
    expect(html).toContain("NOT DELIVERED");
  });

  it("marks a queued steer as landing at the next step, not as failed", () => {
    // A provider that cannot take input mid-turn has the guidance durably
    // queued. That is a success with a delay, not a failure.
    const queued = event(37, "delegation.steered", {
      itemId: "steer-3", role: "system", status: "next_turn_boundary", title: "You steered Implementation · strong",
      text: "use the existing store",
      data: { childSessionId: "w1", label: "Implementation · strong", steeredBy: "user", steerDelivered: true, landed: "next_turn_boundary", orchestratorNotified: true },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[queued]}/>);
    expect(html).toContain("AT NEXT STEP");
    expect(html).not.toContain("NOT DELIVERED");
  });

  it("keeps a landed steer out of the failure state when only the orchestrator missed the notice", () => {
    const unnotified = event(38, "delegation.steered", {
      itemId: "steer-4", role: "system", status: "now", title: "You steered Implementation · strong",
      data: { childSessionId: "w1", label: "Implementation · strong", steeredBy: "user", steerDelivered: true, landed: "now", orchestratorNotified: false },
    });
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[unnotified]}/>);
    expect(html).not.toContain("NOT DELIVERED");
    expect(html).toContain("ORCHESTRATOR NOT TOLD");
  });

  it("surfaces projected continuation fidelity with stronger mid-turn warning", () => {
    const boundary = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} continuationFidelity="projected_at_boundary"/>);
    expect(boundary).toContain("phase-boundary projection");
    expect(boundary).toContain('role="status"');
    const midTurn = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} continuationFidelity="projected_mid_turn"/>);
    expect(midTurn).toContain("Continuation fidelity degraded");
    expect(midTurn).toContain('role="alert"');
  });
});
