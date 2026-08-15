import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { AgentEvent, CompletionSummary, Session, SessionEntry } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" };
const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides });
const completion = (verdict: CompletionSummary["verdict"]): CompletionSummary => ({ attemptId:"a",contractId:"c",verdict,repository:{head:"abcdef1234567890",dirtyDigest:"clean"},passedRequired:0,totalRequired:1,markdownCommitted:false,waiverReason:verdict === "waived" ? "Accepted risk" : null,checks:[{checkId:"gate",kind:"deterministic",required:true,status:verdict === "verified" ? "passed" : verdict === "changes_requested" ? "failed" : verdict === "superseded" ? "stale" : verdict === "waived" ? "skipped" : "pending",executor:"bridge.shell",command:"bun test",verifierFamily:null,detail:null,outputDigest:verdict === "verified" ? "digest" : null,artifactRefs:[]}] });

describe("AgentConversation", () => {
  it("shows revision-bound verification without requiring a committed contract file", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} completion={{ attemptId:"a",contractId:"c",verdict:"waived",repository:{head:"abcdef1234567890",dirtyDigest:"clean"},passedRequired:1,totalRequired:2,markdownCommitted:false,waiverReason:"Browser unavailable",checks:[{checkId:"tests",kind:"deterministic",required:true,status:"passed",executor:"bridge.shell",command:"bun test",verifierFamily:null,detail:"159 passed",outputDigest:"d",artifactRefs:[]},{checkId:"journey",kind:"user_testing",required:true,status:"skipped",executor:"bridge.worker",command:null,verifierFamily:"claude",detail:"No browser",outputDigest:null,artifactRefs:[]}]} } />);
    expect(html).toContain("Verified with waiver");
    expect(html).toContain("private contract");
    expect(html).toContain("abcdef123456");
    expect(html).toContain("Browser unavailable");
    expect(html).toContain("skipped");
  });

  it.each([
    ["verifying", "Verifying", "pending"],
    ["changes_requested", "Changes requested", "failed"],
    ["verified", "Verified", "passed"],
    ["superseded", "Evidence superseded", "stale"],
    ["waived", "Verified with waiver", "skipped"],
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
  it("surfaces conversation and file-state divergence", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[]} repositoryDivergence="diverged"/>);
    expect(html).toContain("This branch&#x27;s context predates the current file state.");
    expect(html).toContain('role="alert"');
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
