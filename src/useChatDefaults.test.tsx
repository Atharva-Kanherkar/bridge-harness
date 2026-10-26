// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { useResolvedChatDraft } from "./useChatDefaults";
import type { ChatDefaults, ResolvedChatSettings } from "./protocol/generated/protocol";

let host:HTMLDivElement;let root:ReturnType<typeof createRoot>;
beforeEach(()=>{(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(()=>{act(()=>root.unmount());host.remove();vi.restoreAllMocks();});
function settings(workspaceId:string):ResolvedChatSettings{return {workspaceId,projectId:workspaceId,revision:{application:0,project:1},target:{kind:"direct"},model:{harness:"codex",model:workspaceId},effort:null,access:"ask",createWorktree:false,activeTurnInput:"steer",sources:{}};}
function Harness({workspace,overrides={},enabled=true}:{workspace:string;overrides?:ChatDefaults;enabled?:boolean}){const result=useResolvedChatDraft(workspace,overrides,enabled);return <div>{result.loading?"loading":result.error??result.settings?.model.model}</div>;}
it("ignores a project response that arrives after a newer project's response",async()=>{
  let a!:(s:ResolvedChatSettings)=>void;let b!:(s:ResolvedChatSettings)=>void;
  const resolve=vi.spyOn(bridgeApi,"resolveChatSettings").mockImplementationOnce(()=>new Promise(r=>{a=r;})).mockImplementationOnce(()=>new Promise(r=>{b=r;}));
  await act(async()=>root.render(<Harness workspace="a"/>));await act(async()=>root.render(<Harness workspace="b"/>));
  expect(host.textContent).toBe("loading");await act(async()=>b(settings("b")));expect(host.textContent).toBe("b");
  await act(async()=>a(settings("a")));expect(host.textContent).toBe("b");expect(resolve).toHaveBeenCalledTimes(2);
});
it("revalidates explicit edits without accepting an old response for the same project",async()=>{
  let old!:(s:ResolvedChatSettings)=>void;
  vi.spyOn(bridgeApi,"resolveChatSettings").mockImplementationOnce(()=>new Promise(r=>{old=r;})).mockResolvedValueOnce(settings("new-model"));
  await act(async()=>root.render(<Harness workspace="a"/>));await act(async()=>root.render(<Harness workspace="a" overrides={{model:{harness:"codex",model:"new-model"}}}/>));
  await act(async()=>old(settings("old-model")));expect(host.textContent).toBe("new-model");
});
it("reports host validation failures without inventing a fallback",async()=>{
  vi.spyOn(bridgeApi,"resolveChatSettings").mockRejectedValue(new Error("Model is unavailable"));
  await act(async()=>root.render(<Harness workspace="a"/>));expect(host.textContent).toBe("Model is unavailable");
});
it("does not resolve or launch anything when the draft surface is disabled",async()=>{
  const resolve=vi.spyOn(bridgeApi,"resolveChatSettings");const create=vi.spyOn(bridgeApi,"createConfiguredChat");
  await act(async()=>root.render(<Harness workspace="a" enabled={false}/>));expect(resolve).not.toHaveBeenCalled();expect(create).not.toHaveBeenCalled();
});
