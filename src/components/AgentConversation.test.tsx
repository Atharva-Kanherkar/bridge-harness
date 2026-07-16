import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { AgentEvent, CompletionSummary, Session, SessionEntry } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh" };
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
