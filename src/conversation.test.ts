import { describe, expect, it } from "vitest";
import { reduceConversation } from "./conversation";
import type { AgentEvent } from "./types";

const event = (id:number,kind:string,overrides:Partial<AgentEvent>={}):AgentEvent => ({ id,sessionId:"s",sequence:id,protocolVersion:1,kind,itemId:null,role:null,status:null,title:null,text:null,data:{},providerMeta:{},createdAt:"now",...overrides });

describe("normalized conversation reducer",()=>{
  it("assembles streaming assistant messages",()=>{const items=reduceConversation([event(1,"message.delta",{itemId:"m",role:"assistant",text:"hel"}),event(2,"message.delta",{itemId:"m",role:"assistant",text:"lo"}),event(3,"message.completed",{itemId:"m",role:"assistant",text:"hello",status:"completed"})]);expect(items).toHaveLength(1);expect(items[0].text).toBe("hello");expect(items[0].status).toBe("completed");});
  it("tracks approval resolution by normalized event id",()=>{const items=reduceConversation([event(7,"approval.requested",{title:"Approve command",status:"pending"}),event(8,"approval.resolved",{data:{requestEventId:7,decision:"accept"}})]);expect(items[0].type).toBe("approval");expect(items[0].status).toBe("accept");});
  it("ignores unknown provider events without losing following items",()=>{const items=reduceConversation([event(1,"provider.unknown"),event(2,"message.completed",{itemId:"m",role:"assistant",text:"safe"})]);expect(items).toHaveLength(1);expect(items[0].text).toBe("safe");});
  it("reduces a tool lifecycle and streams command output",()=>{const items=reduceConversation([event(1,"command.started",{itemId:"c",title:"bun test",status:"inProgress",data:{type:"commandExecution"}}),event(2,"command.output_delta",{itemId:"c",text:"pass 1\n",status:"inProgress"}),event(3,"command.output_delta",{itemId:"c",text:"pass 2\n",status:"inProgress"}),event(4,"command.completed",{itemId:"c",title:"bun test",status:"completed",data:{type:"commandExecution",aggregatedOutput:"pass 1\npass 2\n"}})]);expect(items).toHaveLength(1);expect(items[0].status).toBe("completed");expect(items[0].data.aggregatedOutput).toContain("pass 2");});
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
});
