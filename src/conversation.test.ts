import { describe, expect, it } from "vitest";
import { projectSessionConversation, reduceConversation, selectActiveBranch } from "./conversation";
import type { AgentEvent, SessionEntry } from "./types";

const event = (id:number,kind:string,overrides:Partial<AgentEvent>={}):AgentEvent => ({ id,sessionId:"s",sequence:id,protocolVersion:1,kind,itemId:null,role:null,status:null,title:null,text:null,data:{},providerMeta:{},createdAt:"now",...overrides });
const entry = (id:string,parentEntryId:string|null,kind:string,payload:Record<string,unknown>={},sequence=Number(id.replace(/\D/g,""))||1,overrides:Partial<SessionEntry>={}):SessionEntry => ({ id,sessionId:"s",parentEntryId,sequence,kind,payload,providerEventId:null,contextVisibility:"eligible",tokenEstimate:null,createdAt:"now",...overrides });

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
});
