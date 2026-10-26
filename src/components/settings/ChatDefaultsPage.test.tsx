// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor, AgentDefinition, BridgeState } from "../../types";
import { ChatDefaultsPage } from "./ChatDefaultsPage";
import type { ChatDefaultsView } from "../../protocol/generated/protocol";

let host:HTMLDivElement;let root:ReturnType<typeof createRoot>;
const adapter={id:"codex",label:"Codex",available:true,defaultModel:"standard",models:[{id:"standard",label:"Standard",supportedEffortLevels:["low","high"],available:true,compatible:true}]} as AdapterDescriptor;
const view=():ChatDefaultsView=>({application:{revision:2,defaults:{access:"full",model:{harness:"codex",model:"standard"}}},project:{revision:3,defaults:{}}});
beforeEach(()=>{(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;host=document.createElement("div");document.body.append(host);root=createRoot(host);vi.spyOn(bridgeApi,"state").mockResolvedValue({projects:[{id:"p",name:"Project A"},{id:"q",name:"Project B"}],workspaces:[],sessions:[],events:[]} as unknown as BridgeState);vi.spyOn(bridgeApi,"getChatDefaults").mockResolvedValue(view());});
afterEach(()=>{act(()=>root.unmount());host.remove();vi.restoreAllMocks();});
async function render(){await act(async()=>root.render(<ChatDefaultsPage adapters={[adapter]} agents={[]}/>));}
async function choose(label:string,text:string){await act(async()=>host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());const option=[...document.querySelectorAll<HTMLElement>('[role="option"]')].find(o=>o.textContent===text);expect(option,`${label}: ${text}`).toBeTruthy();await act(async()=>{option!.dispatchEvent(new MouseEvent("pointerdown",{bubbles:true}));option!.click();});}
async function click(text:string){const button=[...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent===text);expect(button).toBeTruthy();await act(async()=>button!.click());}
it("saves a project override with the acknowledged project revision and leaves inherited fields absent",async()=>{
  const save=vi.spyOn(bridgeApi,"saveChatDefaults").mockResolvedValue({...view(),project:{revision:4,defaults:{access:"ask"}}});
  const create=vi.spyOn(bridgeApi,"createConfiguredChat");await render();await choose("Defaults scope","Project A");
  expect(host.textContent).toContain("Inherits application default");await choose("Default chat access","User approval");await click("Save defaults");
  expect(save).toHaveBeenCalledWith({projectId:"p",expectedRevision:3,defaults:{access:"ask"}});expect(host.querySelector('[role="status"]')?.textContent).toBe("Saved");expect(create).not.toHaveBeenCalled();
});
it("reports save failures without claiming Saved and keeps the user's draft",async()=>{
  vi.spyOn(bridgeApi,"saveChatDefaults").mockRejectedValue(new Error("Disk is full"));await render();await choose("Default chat access","User approval");await click("Save defaults");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Disk is full");expect(host.textContent).not.toContain("Saved");expect(host.querySelector('[aria-label="Default chat access"]')?.textContent).toContain("User approval");
});
it("reset clears project overrides with a checked revision",async()=>{
  const save=vi.spyOn(bridgeApi,"saveChatDefaults").mockResolvedValue(view());await render();await choose("Defaults scope","Project A");await click("Reset to inherited defaults");expect(save).toHaveBeenCalledWith({projectId:"p",expectedRevision:3,defaults:{}});
});
it("does not let a late project read overwrite another project's editor",async()=>{
  await render();let p!:(v:ChatDefaultsView)=>void;let q!:(v:ChatDefaultsView)=>void;
  vi.mocked(bridgeApi.getChatDefaults).mockImplementationOnce(()=>new Promise(r=>{p=r;})).mockImplementationOnce(()=>new Promise(r=>{q=r;}));
  await choose("Defaults scope","Project A");await choose("Defaults scope","Project B");
  const save=vi.spyOn(bridgeApi,"saveChatDefaults").mockResolvedValue({...view(),project:{revision:10,defaults:{access:"ask"}}});
  await act(async()=>q({...view(),project:{revision:9,defaults:{access:"ask"}}}));await act(async()=>p({...view(),project:{revision:5,defaults:{access:"full"}}}));
  await click("Save defaults");expect(save.mock.calls[0][0]).toMatchObject({projectId:"q",expectedRevision:9,defaults:{access:"ask"}});
});
it("labels inherited values with the same words as the menus, never raw data",async()=>{
  await render();await choose("Defaults scope","Project A");
  expect(host.textContent).toContain("Inherits application default: Codex · Standard");expect(host.textContent).toContain("Inherits application default: Full access");
  expect(host.textContent).not.toContain('"harness"');
});
it("offers a specialist only the thinking efforts a worker can start",async()=>{
  const wide={...adapter,models:[{...adapter.models[0],supportedEffortLevels:["high","max"]}]} as AdapterDescriptor;
  const research={id:"research",name:"Research agent",role:"research",enabled:true} as AgentDefinition;
  await act(async()=>root.render(<ChatDefaultsPage adapters={[wide]} agents={[research]}/>));
  const efforts=async()=>{await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="Default thinking effort"]')!.click());const values=[...document.querySelectorAll<HTMLElement>('[role="option"]')].map(o=>o.textContent);await act(async()=>document.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true})));return values;};
  expect(await efforts()).toContain("max");
  await choose("Default chat target","Research agent");
  const offered=await efforts();expect(offered).toContain("high");expect(offered).not.toContain("max");
});
