import { describe, expect, it } from "vitest";
import { projectSessionConversation, reduceConversation, selectActiveBranch } from "./conversation";
import type { AgentEvent, SessionEntry } from "./types";

const event = (id:number,kind:string,overrides:Partial<AgentEvent>={}):AgentEvent => ({ id,sessionId:"s",sequence:id,protocolVersion:1,kind,itemId:null,role:null,status:null,title:null,text:null,data:{},providerMeta:{},createdAt:"now",...overrides });
const entry = (id:string,parentEntryId:string|null,kind:string,payload:Record<string,unknown>={},sequence=Number(id.replace(/\D/g,""))||1,overrides:Partial<SessionEntry>={}):SessionEntry => ({ id,sessionId:"s",parentEntryId,sequence,semanticSchemaVersion:2,kind,payload,providerEventId:null,contextVisibility:"eligible",tokenEstimate:null,createdAt:"now",...overrides });

describe("normalized conversation reducer",()=>{
  it("assembles streaming assistant messages",()=>{const items=reduceConversation([event(1,"message.delta",{itemId:"m",role:"assistant",text:"hel"}),event(2,"message.delta",{itemId:"m",role:"assistant",text:"lo"}),event(3,"message.completed",{itemId:"m",role:"assistant",text:"hello",status:"completed"})]);expect(items).toHaveLength(1);expect(items[0].text).toBe("hello");expect(items[0].status).toBe("completed");});
  it("tracks approval resolution by normalized event id",()=>{const items=reduceConversation([event(7,"approval.requested",{title:"Approve command",status:"pending"}),event(8,"approval.resolved",{data:{requestEventId:7,decision:"accept"}})]);expect(items[0].type).toBe("approval");expect(items[0].status).toBe("accept");});
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
