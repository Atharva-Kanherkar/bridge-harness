import { describe, expect, it } from "vitest";
import { attachmentUris, compactionReasonLabel, delegationChildSessionId, undeliveredPending, delegationFacet, foldWorkerDelegations, isInternalCompactionEnvelope, projectSessionConversation, reduceConversation, selectActiveBranch, toolCallDisplay, type ConversationItem } from "./conversation";
import type { AgentEvent, SessionEntry } from "./types";

const event = (id:number,kind:string,overrides:Partial<AgentEvent>={}):AgentEvent => ({ id,sessionId:"s",sequence:id,protocolVersion:1,kind,itemId:null,role:null,status:null,title:null,text:null,data:{},providerMeta:{},createdAt:"now",...overrides });
const entry = (id:string,parentEntryId:string|null,kind:string,payload:Record<string,unknown>={},sequence=Number(id.replace(/\D/g,""))||1,overrides:Partial<SessionEntry>={}):SessionEntry => ({ id,sessionId:"s",parentEntryId,sequence,semanticSchemaVersion:2,kind,payload,providerEventId:null,contextVisibility:"eligible",tokenEstimate:null,createdAt:"now",...overrides });

describe("normalized conversation reducer",()=>{
  it("assembles streaming assistant messages",()=>{const items=reduceConversation([event(1,"message.delta",{itemId:"m",role:"assistant",text:"hel"}),event(2,"message.delta",{itemId:"m",role:"assistant",text:"lo"}),event(3,"message.completed",{itemId:"m",role:"assistant",text:"hello",status:"completed"})]);expect(items).toHaveLength(1);expect(items[0].text).toBe("hello");expect(items[0].status).toBe("completed");});
  it("tracks approval resolution by normalized event id",()=>{const items=reduceConversation([event(7,"approval.requested",{title:"Approve command",status:"pending"}),event(8,"approval.resolved",{data:{requestEventId:7,decision:"accept"}})]);expect(items[0].type).toBe("approval");expect(items[0].status).toBe("accept");});
  it("keeps permissions and questions as distinct interaction types",()=>{
    const items=reduceConversation([
      event(7,"permission.requested",{title:"Run command",status:"pending",data:{actions:[{decision:"accept",label:"Allow once"}]}}),
      event(8,"permission.resolving",{status:"settling",data:{requestEventId:7,resolvedBy:"policy",reason:"Auto-approve provider permissions"}}),
      event(9,"question.requested",{title:"Choose target",status:"pending",data:{questions:[{id:"target",question:"Which target?"}]}}),
    ]);
    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({type:"permission",status:"settling",data:{resolvedBy:"policy"}});
    expect(items[1]).toMatchObject({type:"question",status:"pending"});
  });
  it("ignores unknown provider events without losing following items",()=>{const items=reduceConversation([event(1,"provider.unknown"),event(2,"message.completed",{itemId:"m",role:"assistant",text:"safe"})]);expect(items).toHaveLength(1);expect(items[0].text).toBe("safe");});
  it("reduces a tool lifecycle and streams command output",()=>{const items=reduceConversation([event(1,"command.started",{itemId:"c",title:"bun test",status:"inProgress",data:{type:"commandExecution"}}),event(2,"command.output_delta",{itemId:"c",text:"pass 1\n",status:"inProgress"}),event(3,"command.output_delta",{itemId:"c",text:"pass 2\n",status:"inProgress"}),event(4,"command.completed",{itemId:"c",title:"bun test",status:"completed",data:{type:"commandExecution",aggregatedOutput:"pass 1\npass 2\n"}})]);expect(items).toHaveLength(1);expect(items[0].status).toBe("completed");expect(items[0].data.aggregatedOutput).toContain("pass 2");});
  it("renders delegation spawn and result as delegation items",()=>{const items=reduceConversation([event(1,"delegation.spawned",{itemId:"spawn-x",role:"system",title:"Delegated to Claude · Fable",text:"do it",data:{model:"fable",modelLabel:"Fable",effort:"high",childSessionId:"x"}}),event(2,"delegation.result",{itemId:"result-x",role:"system",title:"Worker result",text:"done",data:{childSessionId:"x",delivered:true}})]);expect(items).toHaveLength(2);expect(items[0].type).toBe("delegation");expect(items[1].type).toBe("delegation");});
  it("ignores empty reasoning and empty message shells",()=>{
    const items=reduceConversation([
      event(1,"reasoning.delta",{itemId:"r",text:""}),
      event(2,"reasoning.completed",{itemId:"r",text:"",status:"completed"}),
      event(3,"message.delta",{itemId:"m",role:"assistant",text:"Hi"}),
      event(4,"message.completed",{itemId:"m",role:"assistant",text:"Hi there",status:"completed"})
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].text).toBe("Hi there");
  });
  it("hides worker result blocks without mutating raw events",()=>{
    const raw = "Finished the work.\n```bridge-worker-result\n{\"schemaVersion\":1,\"status\":\"completed\"}\n```\nReview the summary.";
    const source = event(1,"message.completed",{itemId:"worker-result",role:"assistant",text:raw,status:"completed"});
    const items = reduceConversation([source]);
    expect(items).toHaveLength(1);
    expect(items[0].text).toBe("Finished the work.\nReview the summary.");
    expect(items[0].text).not.toContain("bridge-worker-result");
    expect(source.text).toBe(raw);
  });
});

describe("session forest conversation projection",()=>{
  const root=entry("e1",null,"user.message",{text:"start"},1);
  const fork=entry("e2","e1","assistant.message",{text:"shared"},2);
  const left=entry("e3","e2","assistant.message",{text:"left"},3);
  const right=entry("e4","e2","assistant.message",{text:"right"},4);
  const rightTail=entry("e5","e4","assistant.message",{text:"right tail"},5);
  const forest=[rightTail,left,root,right,fork];

  it("selects the root-to-active-leaf path and excludes inactive descendants",()=>{
    expect(selectActiveBranch(forest,"e5").map(({id})=>id)).toEqual(["e1","e2","e4","e5"]);
    expect(projectSessionConversation(forest,"e3").map(({text})=>text)).toEqual(["start","shared","left"]);
  });

  it("preserves entry-derived keys when the selected leaf changes",()=>{
    const leftKeys=projectSessionConversation(forest,"e3").map(({key})=>key);
    const rightKeys=projectSessionConversation(forest,"e5").map(({key})=>key);
    expect(leftKeys.slice(0,2)).toEqual(["entry:e1","entry:e2"]);
    expect(rightKeys.slice(0,2)).toEqual(leftKeys.slice(0,2));
  });

  it("projects current and N-1 semantic event schemas equivalently",()=>{
    const current=entry("current",null,"assistant.message",{text:"stable"},1,{semanticSchemaVersion:2});
    const previous=entry("previous",null,"assistant.message",{text:"stable"},1,{semanticSchemaVersion:1});
    const currentItem=projectSessionConversation([current],"current")[0];
    const previousItem=projectSessionConversation([previous],"previous")[0];
    expect({...currentItem,key:"entry",entryId:"entry"}).toEqual({...previousItem,key:"entry",entryId:"entry"});
  });

  it("fails closed on an unsupported future semantic event schema",()=>{
    const future=entry("future",null,"assistant.message",{text:"do not guess"},1,{semanticSchemaVersion:3});
    expect(()=>projectSessionConversation([future],"future")).toThrow("Unsupported semantic event schema version 3");
  });

  it("renders a model switch with the fidelity it reports",()=>{
    const items=projectSessionConversation([
      entry("m1",null,"session.model_changed",{role:"system",status:"ready",title:"Chat model changed",text:"Chat runtime changed from codex/stub-fast to claude/opus. The next message starts a fresh provider session — carried forward: summary + 2 decisions + 3 files.",data:{previousHarness:"codex",harness:"claude",model:"opus",freshProviderSession:true,carriedContext:{summary:true,decisions:2,filesTouched:3,recentEntries:4}}},1),
    ],"m1");
    expect(items).toHaveLength(1);
    expect(items[0].type).toBe("activity");
    expect(items[0].title).toBe("Chat model changed");
    expect(items[0].text).toContain("carried forward: summary + 2 decisions + 3 files");
  });

  it("renders a carried handoff brief inside the new chat's transcript",()=>{
    const items=projectSessionConversation([
      entry("h1",null,"handoff.brief",{text:"Bridge checkpoint-restoration context (stored history, not native provider resume):\nuser.message: we chose the SQLite token store",sourceSessionId:"source",sourceHarness:"claude"},1),
      entry("h2","h1","user.message",{text:"what store did we pick?"},2),
    ],"h2");
    expect(items).toHaveLength(2);
    expect(items[0].type).toBe("activity");
    expect(items[0].title).toBe("Handoff brief");
    expect(items[0].text).toContain("SQLite token store");
    expect(items[1].text).toBe("what store did we pick?");
  });

  it("maps checkpoint, compaction, and branch summary entries to dedicated cards",()=>{
    const cards=[
      entry("e1",null,"checkpoint",{summary:"Saved state"},1),
      entry("e2","e1","compaction",{summary:"Reduced context"},2),
      entry("e3","e2","branch.summary",{summary:"Retained boundary"},3),
    ];
    const items=projectSessionConversation(cards,"e3");
    expect(items.map(({type})=>type)).toEqual(["checkpoint","compaction","branch-summary"]);
    expect(items.map(({text})=>text)).toEqual(["Saved state","Reduced context","Retained boundary"]);
  });

  it("folds durable approval resolution into the pending request card",()=>{
    const request=entry("e6",null,"approval.requested",{status:"pending",approvalId:"scope",approvalType:"delegation_path_scope",title:"Approve scope"},6);
    const resolved=entry("e7","e6","approval.resolved",{approvalId:"scope",requestEventId:6,decision:"accept"},7);
    const items=projectSessionConversation([resolved,request],"e7");
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({type:"approval",eventId:6,status:"accept",title:"Approve scope"});
  });

  it("folds provider-shaped durable approval resolution data",()=>{
    const request=entry("e8",null,"approval.requested",{status:"pending",title:"Approve command"},8);
    const resolved=entry("e9","e8","approval.resolved",{status:"completed",data:{requestEventId:8,decision:"decline"}},9);
    const items=projectSessionConversation([request,resolved],"e9");
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({eventId:8,status:"decline",title:"Approve command"});
  });

  it("folds durable permission resolution actor and failure into one card",()=>{
    const request=entry("e10",null,"permission.requested",{status:"pending",title:"Run command",data:{actions:[{decision:"accept",label:"Allow once"}]}},10);
    const resolved=entry("e11","e10","permission.resolved",{status:"failed",data:{requestEventId:10,decision:"accept",resolvedBy:"human",failure:"provider pipe closed"}},11);
    const items=projectSessionConversation([request,resolved],"e11");
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({type:"permission",eventId:10,status:"failed",data:{resolvedBy:"human",failure:"provider pipe closed"}});
  });

  it("keeps raw provider entries collapsed and inspectable",()=>{
    const raw=entry("e2","e1","provider.unknown",{title:"provider frame",text:"opaque",providerMeta:{requestId:"r"}},2,{contextVisibility:"worker_raw"});
    const items=projectSessionConversation([root,raw],"e2");
    expect(items[1]).toMatchObject({key:"entry:e2",type:"raw",title:"provider frame",text:"opaque",data:{collapsed:true,inspectable:true}});
    expect(items[1].data.providerMeta).toEqual({requestId:"r"});
  });

  it("returns an empty projection when the active leaf is unavailable",()=>{
    expect(projectSessionConversation(forest,"missing")).toEqual([]);
    expect(projectSessionConversation(forest,null)).toEqual([]);
  });

  it("projects provider error entries as a top-level error item, not a folded activity",()=>{
    const err=entry("e2","e1","error",{status:"failed",text:"You've hit your usage limit. Try again later.",data:{error:{codexErrorInfo:"usageLimitExceeded"}}},2);
    const items=projectSessionConversation([root,err],"e2");
    expect(items[1]).toMatchObject({type:"error",status:"failed",text:"You've hit your usage limit. Try again later."});
  });

  it("falls back to a nested error message when the entry has no top-level text",()=>{
    const err=entry("e2","e1","error",{status:"failed",data:{error:{message:"rate limit exceeded"}}},2);
    expect(projectSessionConversation([root,err],"e2")[1]).toMatchObject({type:"error",text:"rate limit exceeded"});
  });
});

describe("worker delegation fold",()=>{
  const spawn = (childSessionId:string,sequence=1) => reduceConversation([event(sequence,"delegation.spawned",{itemId:`spawn-${childSessionId}`,role:"system",title:"Delegated to Implementation · strong",text:"add rotation",data:{childSessionId,modelLabel:"Fable",request:{objective:"add rotation"}}})])[0];
  const result = (childSessionId:string,sequence=2,data:Record<string,unknown>={}) => reduceConversation([event(sequence,"delegation.result",{itemId:`result-${sequence}`,role:"system",status:"completed",title:"Worker result",text:"done",data:{childSessionId,delivered:true,status:"completed",...data}})])[0];

  it("names each delegation facet from its own payload",()=>{
    expect(delegationFacet(spawn("x"))).toBe("spawn");
    expect(delegationFacet(result("x"))).toBe("result");
    expect(delegationFacet({...spawn("x"),data:{childBlocked:true}})).toBe("blocked");
    expect(delegationFacet({...spawn("x"),data:{willRetry:false}})).toBe("rejected");
    expect(delegationFacet({...spawn("x"),data:{steeredBy:"user",delivered:true}})).toBe("steered");
    expect(delegationChildSessionId(spawn("x"))).toBe("x");
    expect(delegationChildSessionId({...spawn("x"),data:{}})).toBeUndefined();
  });

  it("folds a worker result into the panel that spawned it",()=>{
    const folded = foldWorkerDelegations([spawn("x"), result("x")]);
    expect(folded).toHaveLength(1);
    // Both halves survive: the spawn's routing detail and the result's outcome.
    expect(folded[0].data.modelLabel).toBe("Fable");
    expect(folded[0].data.delivered).toBe(true);
    expect(folded[0].text).toBe("done");
    expect(folded[0].key).toBe(spawn("x").key);
  });

  it("keeps each worker's panel separate",()=>{
    const folded = foldWorkerDelegations([spawn("x",1), spawn("y",2), result("y",3), result("x",4)]);
    expect(folded).toHaveLength(2);
    expect(folded.map(item => item.data.childSessionId)).toEqual(["x","y"]);
    expect(folded.every(item => item.data.delivered === true)).toBe(true);
  });

  it("leaves an orphan worker result visible",()=>{
    // Durable history truncated away the spawn, or the branch moved: the
    // outcome must still be readable rather than folded into nothing.
    const folded = foldWorkerDelegations([result("x")]);
    expect(folded).toHaveLength(1);
    expect(folded[0].data.delivered).toBe(true);
  });

  it("does not fold a steer, a block, or a rejection onto the panel",()=>{
    const steered = {...result("x",3),data:{childSessionId:"x",steeredBy:"user",delivered:true,label:"Implementation"}};
    const folded = foldWorkerDelegations([spawn("x"), steered]);
    expect(folded).toHaveLength(2);
    expect(folded[0].data.steeredBy).toBeUndefined();
  });

  it("carries a classified failure onto the panel so its retry action survives",()=>{
    const failed = result("x",2,{failureCause:"the worker stopped responding",failureClass:"stalled",canRetry:true});
    const folded = foldWorkerDelegations([spawn("x"), failed]);
    expect(folded).toHaveLength(1);
    expect(folded[0].data.failureCause).toBe("the worker stopped responding");
    expect(folded[0].data.canRetry).toBe(true);
  });

  it("passes non-delegation items through untouched",()=>{
    const items = reduceConversation([event(1,"message.completed",{itemId:"m",role:"assistant",text:"hi",status:"completed"})]);
    expect(foldWorkerDelegations(items)).toEqual(items);
  });

  it("folds a durable spawn together with a live result",()=>{
    const durable = projectSessionConversation([entry("e1",null,"delegation.spawned",{itemId:"spawn-x",title:"Delegated to Implementation",data:{childSessionId:"x"}},1)],"e1");
    const folded = foldWorkerDelegations([...durable, result("x",2)]);
    expect(folded).toHaveLength(1);
    expect(folded[0].entryId).toBe("e1");
    expect(folded[0].data.delivered).toBe(true);
  });
});

/* ── Tool-call display data ─────────────────────────────────────────────── */

const call = (overrides: Partial<ConversationItem> = {}): ConversationItem => ({
  key: "k", type: "activity", eventId: 1, sequence: 1, text: "", data: {}, ...overrides,
});

describe("toolCallDisplay", () => {
  it("reads a Claude bash call as a command", () => {
    const display = toolCallDisplay(call({ data: { name: "Bash", input: { command: "bun run test" } } }));
    expect(display.verb).toBe("run");
    expect(display.glyph).toBe("terminal");
    expect(display.command).toBe("bun run test");
  });

  it("splits a file path into a target and a path", () => {
    const display = toolCallDisplay(call({ data: { name: "Edit", input: { file_path: "src-tauri/src/lib.rs" } } }));
    expect(display.verb).toBe("edit");
    expect(display.target).toBe("lib.rs");
    expect(display.path).toBe("src-tauri/src/lib.rs");
  });

  it("names a write distinctly from an edit", () => {
    expect(toolCallDisplay(call({ data: { name: "Write", input: { file_path: "a/b.rs" } } })).done).toBe("Wrote");
  });

  it("reads a Codex command execution", () => {
    const display = toolCallDisplay(call({ title: "bun test", data: { type: "commandExecution", command: "bun test" } }));
    expect(display.verb).toBe("run");
    expect(display.command).toBe("bun test");
  });

  describe("exit codes", () => {
    it("reads camelCase", () => {
      expect(toolCallDisplay(call({ data: { type: "commandExecution", command: "x", exitCode: 0 } })).exitCode).toBe(0);
    });

    it("tolerates snake_case", () => {
      expect(toolCallDisplay(call({ data: { type: "commandExecution", command: "x", exit_code: 2 } })).exitCode).toBe(2);
    });

    it("looks inside a nested provider state", () => {
      expect(toolCallDisplay(call({ data: { name: "Bash", state: { metadata: { exit: 127 } } } })).exitCode).toBe(127);
    });

    it("stays undefined when nothing reports one", () => {
      // The chip has to degrade to nothing rather than render "exit NaN".
      expect(toolCallDisplay(call({ data: { type: "commandExecution", command: "x" } })).exitCode).toBeUndefined();
    });

    it("ignores an unparseable value", () => {
      expect(toolCallDisplay(call({ data: { type: "commandExecution", command: "x", exitCode: "boom" } })).exitCode).toBeUndefined();
    });
  });

  describe("patches", () => {
    const PATCH = "@@ -1,2 +1,2 @@\n-a\n+b";

    it("takes the diff a file change carries", () => {
      expect(toolCallDisplay(call({ type: "diff", data: { path: "a.rs", patch: PATCH } })).patch).toBe(PATCH);
    });

    it("joins per-file diffs in order", () => {
      const display = toolCallDisplay(call({
        type: "diff",
        data: { changes: [{ path: "a.rs", diff: PATCH }, { path: "b.rs", diff: "@@ -9 +9 @@\n+c" }] },
      }));
      expect(display.patch).toBe(`${PATCH}\n@@ -9 +9 @@\n+c`);
      expect(display.path).toBe("a.rs");
    });

    it("falls back to the body when the body is unmistakably a diff", () => {
      expect(toolCallDisplay(call({ type: "diff", text: PATCH, data: { path: "a.rs" } })).patch).toBe(PATCH);
    });

    it("does not claim a patch from prose that merely has plus signs", () => {
      expect(toolCallDisplay(call({ type: "diff", text: "+1 more thing\n+another", data: {} })).patch).toBeUndefined();
    });

    it("never claims one for a read, whose output is only ever output", () => {
      const display = toolCallDisplay(call({ data: { name: "Read", input: { file_path: "a.rs" } }, text: PATCH }));
      expect(display.patch).toBeUndefined();
      expect(display.output).toBe(PATCH);
    });
  });

  describe("diffstat and duration", () => {
    it("surfaces numbers", () => {
      const display = toolCallDisplay(call({ type: "diff", data: { additions: 24, deletions: 3, durationMs: 400 } }));
      expect(display.additions).toBe(24);
      expect(display.deletions).toBe(3);
      expect(display.durationMs).toBe(400);
    });

    it("leaves them undefined when absent", () => {
      const display = toolCallDisplay(call({ type: "diff", data: {} }));
      expect(display.additions).toBeUndefined();
      expect(display.durationMs).toBeUndefined();
    });
  });

  describe("status", () => {
    it.each([
      ["inProgress", "running"],
      ["streaming", "running"],
      ["failed", "failed"],
      ["error", "failed"],
      ["completed", "completed"],
      [undefined, "idle"],
      ["pending", "idle"],
    ] as const)("reads %s as %s", (status, expected) => {
      expect(toolCallDisplay(call({ status, data: {} })).status).toBe(expected);
    });
  });

  it("reads a durable file change as an edit carrying its diff", () => {
    // The durable projection used to type `file_change.*` as plain activity, so
    // a patch replayed from history came back as a generic tool row with no
    // diff. The live reducer always typed it as one; both agree now.
    const patch = "@@ -1,2 +1,2 @@\n-a\n+b";
    const [item] = projectSessionConversation(
      [entry("e1", null, "file_change.completed", { status: "completed", title: "lib.rs", data: { path: "src/lib.rs", additions: 2, deletions: 1, patch } }, 1)],
      "e1",
    );
    expect(item.type).toBe("diff");
    const display = toolCallDisplay(item);
    expect(display.verb).toBe("edit");
    expect(display.target).toBe("lib.rs");
    expect(display.patch).toBe(patch);
  });

  // A maintenance reason is a wire value; a card that shows one to the reader is
  // showing plumbing. Switching models used to put `before_downgrade` on screen.
  it("says a compaction reason in English, and leaves an unknown one alone", () => {
    const [requested] = projectSessionConversation(
      [entry("e1", null, "compaction.requested", { reason: "before_downgrade", attempt: 0 }, 1)],
      "e1",
    );
    expect(requested.text).toBe("Before switching models");
    expect(requested.text).not.toContain("before_downgrade");
    expect(compactionReasonLabel("context_pressure")).toBe("Context was nearly full");
    // The host owns this set and may add to it: a raw value read once beats a
    // wrong value read confidently.
    expect(compactionReasonLabel("some_future_reason")).toBe("some_future_reason");
    expect(compactionReasonLabel(undefined)).toBe("");
  });

  it("projects classified compaction copy while retaining the diagnostic", () => {
    const [failed] = projectSessionConversation(
      [entry("e1", null, "compaction.failed", {
        reason: "checkpoint metadata does not match its controller request",
        message: "Bridge could not verify the provider's checkpoint, so no conversation history was replaced.",
        retryable: true,
      }, 1)],
      "e1",
    );
    expect(failed.status).toBe("failed");
    expect(failed.text).toContain("could not verify");
    expect(failed.text).not.toContain("metadata does not match");
    expect(failed.data.reason).toBe("checkpoint metadata does not match its controller request");
  });

  it("suppresses checkpoint output only when backend origin or persisted state identifies it", () => {
    const payload = JSON.stringify({
      schemaVersion: 1,
      summary: "Internal summary",
      decisions: [],
      filesTouched: [],
      sourceAgent: "session-1",
      firstRetainedEntryId: "retained-1",
      tokensBefore: 4200,
      reason: "before_downgrade",
    });
    const internal = { bridgeInternalOrigin: "compaction" };
    expect(isInternalCompactionEnvelope(payload, internal)).toBe(true);
    expect(isInternalCompactionEnvelope(`\`\`\`json\n${payload}\n\`\`\``, internal)).toBe(true);
    expect(reduceConversation([
      event(1, "message.completed", { itemId: "internal", role: "assistant", text: payload, data: internal }),
      event(2, "message.completed", { itemId: "safe", role: "assistant", text: '{"answer":42}' }),
    ]).map(item => item.text)).toEqual(['{"answer":42}']);

    const durable = projectSessionConversation([
      entry("e1", null, "compaction.requested", { reason: "before_downgrade" }, 1),
      entry("e2", "e1", "assistant.message", { role: "assistant", text: `\`\`\`json\n${payload}\n\`\`\`` }, 2),
      entry("e3", "e2", "compaction", { summary: "Compacted" }, 3),
      entry("e4", "e3", "assistant.message", { role: "assistant", text: '{"answer":42}' }, 4),
    ], "e4");
    expect(durable.filter(item => item.type === "message").map(item => item.text)).toEqual(['{"answer":42}']);
  });

  it("keeps a complete checkpoint-shaped answer from an ordinary turn", () => {
    const payload = JSON.stringify({
      schemaVersion: 1,
      summary: "Public example",
      decisions: [],
      filesTouched: [],
      sourceAgent: "example",
      firstRetainedEntryId: "entry-1",
      tokensBefore: 42,
      reason: "manual",
    });
    expect(isInternalCompactionEnvelope(payload)).toBe(false);
    expect(reduceConversation([
      event(1, "message.completed", { itemId: "public", role: "assistant", text: payload }),
    ]).map(item => item.text)).toEqual([payload]);
    expect(projectSessionConversation([
      entry("e1", null, "assistant.message", { role: "assistant", text: payload }, 1),
    ], "e1").map(item => item.text)).toEqual([payload]);
  });

  it("does not mistake a title echoed as the body for output", () => {
    expect(toolCallDisplay(call({ title: "bun test", text: "bun test", data: { type: "commandExecution" } })).output).toBeUndefined();
  });
});

describe("attachmentUris", () => {
  it("extracts image data URIs from a persisted user turn payload", () => {
    expect(attachmentUris({
      delivery: "submitted",
      attachments: [{ mediaType: "image/png", dataUri: "data:image/png;base64,AAA" }],
    })).toEqual(["data:image/png;base64,AAA"]);
  });

  it("returns nothing when there are no attachments", () => {
    expect(attachmentUris({ delivery: "submitted" })).toEqual([]);
  });

  it("ignores malformed payloads instead of breaking the transcript row", () => {
    expect(attachmentUris({ attachments: "nope" })).toEqual([]);
    expect(attachmentUris({ attachments: [{ dataUri: "http://not-a-data-uri" }, null, {}] })).toEqual([]);
  });
});

describe("undeliveredPending", () => {
  const userTurn = (id: number, sessionId: string, text: string) =>
    event(id, "message.completed", { sessionId, itemId: `u${id}`, role: "user", text });
  const row = (sessionId: string, text: string) => ({ key: text, sessionId, text });
  const noneSelected = { sessionId: undefined, durableUserTexts: new Set<string>() };

  // The field failure: an aside's pending "hi" checked against the selected
  // session's slice never reconciled, so the aside's startup row counted
  // forever under an already-answered reply.
  it("reconciles each row in its own session, never across sessions", () => {
    const pending = [row("aside-1", "hi"), row("main", "hi")];
    const delivered = undeliveredPending(pending, [userTurn(1, "aside-1", "hi")], noneSelected);
    expect(delivered).toEqual([row("main", "hi")]);
  });

  it("still reconciles the selected session through its durable texts alone", () => {
    const pending = [row("main", "what store did we pick?")];
    const delivered = undeliveredPending(pending, [], { sessionId: "main", durableUserTexts: new Set(["what store did we pick?"]) });
    expect(delivered).toEqual([]);
    // The durable source belongs to the selected session only.
    const other = undeliveredPending([row("aside-1", "what store did we pick?")], [], { sessionId: "main", durableUserTexts: new Set(["what store did we pick?"]) });
    expect(other).toHaveLength(1);
  });

  it("returns the same reference when nothing was delivered", () => {
    const pending = [row("aside-1", "hi")];
    expect(undeliveredPending(pending, [userTurn(1, "main", "hi")], noneSelected)).toBe(pending);
    expect(undeliveredPending([], [], noneSelected)).toEqual([]);
  });

  it("matches on trimmed text, like the optimistic rows it clears", () => {
    const delivered = undeliveredPending([row("s", "  hi  ")], [userTurn(1, "s", "hi")], noneSelected);
    expect(delivered).toEqual([]);
  });
});
