// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

// #350: New chat must open an *unstarted draft* — no session row is created until the
// first message is submitted. Driven through the real App and the api layer's mock
// backend, spying on the two create calls so "created nothing yet" is exact, not a
// count of rail rows that other tests in this process also mutate.

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  // Pin a last-used repo so a New Chat resolves a workspace draft (with a worktree
  // decision), rather than the no-workspace fallback direct chat.
  store.set("bridge.chat.lastWorkspaceId", "demo-1");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container);
    root.render(<App />);
  });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
  const recommended = [...container.querySelectorAll("button")]
    .find(button => button.textContent === "Use recommended defaults");
  if (recommended) {
    await act(async () => recommended.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
  }
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

const byLabel = (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")]
  .find(button => button.getAttribute("aria-label") === label);
const composerField = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Ask Bridge"));
const workModeMenu = () => container.querySelector<HTMLButtonElement>('[aria-label^="Work mode:"]');

async function type(field: HTMLTextAreaElement, text: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setValue.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
function pressEnter(field: HTMLTextAreaElement) {
  field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
}
function spyCreates() {
  return {
    chat: vi.spyOn(bridgeApi, "createChat"),
    workspace: vi.spyOn(bridgeApi, "createWorkspaceSession"),
    configured: vi.spyOn(bridgeApi, "createConfiguredChat"),
  };
}
const totalCreates = (s: ReturnType<typeof spyCreates>) => s.chat.mock.calls.length + s.workspace.mock.calls.length + s.configured.mock.calls.length;

describe("deferred new-chat creation (#350)", () => {
  it("clicking New Chat creates no session and keeps the draft composer", async () => {
    const creates = spyCreates();
    const newChat = byLabel("New Chat");
    expect(newChat, "the rail exposes a New Chat action").toBeTruthy();

    await act(async () => newChat!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // No row was written, and we are still on the unstarted draft surface.
    expect(totalCreates(creates)).toBe(0);
    expect(composerField(), "still on the draft composer").toBeTruthy();

    // Navigating away discards the draft silently — still nothing created.
    const projects = byLabel("Projects");
    if (projects) {
      await act(async () => projects.click());
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    }
    expect(totalCreates(creates)).toBe(0);
  });

  it("rapid repeat clicks on New Chat produce at most one draft and no session", async () => {
    const creates = spyCreates();
    const newChat = byLabel("New Chat")!;
    await act(async () => { newChat.click(); newChat.click(); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    expect(totalCreates(creates)).toBe(0);
    // One draft surface, one composer — not two.
    expect(Array.from(container.querySelectorAll("button")).filter(button => button.textContent?.trim() === "Add project").length).toBe(1);
  });

  it("the first submitted message creates exactly one session and leaves the draft", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    expect(composer, "draft composer present").toBeTruthy();

    await type(composer!, "start the work");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(totalCreates(creates)).toBe(1);
    // Left the welcome/draft surface — the welcome-only "Add project" control is
    // gone because we are now inside the created chat.
    expect(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.trim() === "Add project") ?? null).toBeNull();
  });

  it("an empty submit creates nothing and keeps the draft open", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    // Submit with an empty composer — a chat only exists once it has something in it.
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });

    expect(totalCreates(creates)).toBe(0);
    expect(composerField(), "still on the draft composer").toBeTruthy();
  });

  it("a rapid double submit still creates only one session", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    await type(composer!, "go go go");
    // Two Enters in the same tick — the second must find the create in flight.
    await act(async () => { pressEnter(composer!); pressEnter(composer!); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(totalCreates(creates)).toBe(1);
  });

  it("offers an interactive model picker on the draft and applies the choice on submit", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // The draft is a real chat-in-waiting: an interactive model control, not a
    // display-only "Balanced" badge.
    const modelButton = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.getAttribute("aria-label") ?? "").startsWith("Chat model:"));
    expect(modelButton, "the draft exposes an interactive model picker").toBeTruthy();

    const configured = vi.spyOn(bridgeApi, "createConfiguredChat");
    const updateModel = vi.spyOn(bridgeApi, "updateChatModel");
    await act(async () => modelButton!.click());
    const opus = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").includes("Claude Opus"));
    expect(opus, "the picker lists more than one model").toBeTruthy();
    await act(async () => opus!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // The picker stays open across the pick so model and thinking are set in
    // one go; the footer already shows Opus's ladder.
    const max = container.querySelector<HTMLButtonElement>('[data-effort="max"]')!;
    expect(max, "thinking can be selected before the first message").toBeTruthy();
    await act(async () => max.click());

    const composer = composerField();
    await type(composer!, "with opus please");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(configured).toHaveBeenCalledOnce();
    expect(configured.mock.calls[0][0].overrides).toMatchObject({model:{harness:"claude",model:"opus"},effort:{value:"max"}});
    expect(updateModel).not.toHaveBeenCalled();

  });

  it("the worktree decision is held on the draft and applied on submit, not on selection", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const menu = workModeMenu();
    expect(menu, "the draft surface offers work mode choices").toBeTruthy();
    expect(menu!.getAttribute("aria-label")).toBe("Work mode: Work on branch");

    await act(async () => menu!.click());
    const isolated = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Work mode"] [role="menuitemradio"]')]
      .find(option => option.textContent === "Isolated worktree")!;
    expect(isolated.disabled).toBe(false);
    expect(isolated.getAttribute("aria-checked")).toBe("false");
    await act(async () => isolated.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // Choosing a mode records the decision — it does not start a session.
    expect(totalCreates(creates)).toBe(0);
    expect(workModeMenu()!.getAttribute("aria-label")).toBe("Work mode: Isolated worktree");

    const composer = composerField();
    await type(composer!, "isolate this");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(creates.configured).toHaveBeenCalledTimes(1);
    expect(creates.configured.mock.calls[0][0].overrides.createWorktree).toBe(true);
  });

  const modeRadio = (label: string) => [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="Chat mode"] [role="radio"]')]
    .find(option => option.textContent === label);

  it("a workspace draft starts as an orchestrator unless Direct is chosen", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const orchestrator = modeRadio("Orchestrator");
    expect(orchestrator, "the draft composer offers the chat mode toggle").toBeTruthy();
    expect(orchestrator!.getAttribute("aria-checked")).toBe("true");
    const group = container.querySelector('[role="radiogroup"][aria-label="Chat mode"]')!;
    const hint = document.getElementById(group.getAttribute("aria-describedby")!);
    expect(hint?.textContent).toContain("delegates to workers");

    const creates = spyCreates();
    const composer = composerField();
    await type(composer!, "plan it");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(creates.configured).toHaveBeenCalledTimes(1);
    expect(creates.configured.mock.calls[0][0].overrides.target).toBeUndefined();
  });

  it("Direct mode creates a direct workspace chat that keeps the workspace", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    await act(async () => modeRadio("Direct")!.click());
    expect(modeRadio("Direct")!.getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("with no Bridge orchestration");
    // Choosing a mode records the decision; it does not start a session.
    expect(totalCreates(creates)).toBe(0);

    const composer = composerField();
    await type(composer!, "just answer me");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(creates.chat).not.toHaveBeenCalled();
    expect(creates.workspace).not.toHaveBeenCalled();
    expect(creates.configured).toHaveBeenCalledTimes(1);
    const params=creates.configured.mock.calls[0][0];
    expect(params.workspaceId).toBe("demo-1");
    expect(params.overrides.target).toEqual({kind:"direct"});
    const result:Awaited<ReturnType<typeof bridgeApi.createConfiguredChat>>=await creates.configured.mock.results[0].value;
    const created=result.state.sessions.find(s=>s.id===result.sessionId);
    expect(created?.kind).toBe("direct");

  });
  it("starts a new chat on the extra account picked in the draft", async () => {
    const config = await bridgeApi.configState();
    const claude = config.harnesses.find(harness => harness.id === "claude")!;
    await act(async () => { await bridgeApi.saveHarnessConfig({ ...claude, advanced: { accounts: [{ id: "personal", label: "Personal", homePath: "~/.claude_personal" }] } }); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    const picker = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-label")?.startsWith("Chat model:"))!;
    await act(async () => picker.click());
    const group = [...document.querySelectorAll<HTMLElement>('[role="group"]')].find(item => item.getAttribute("aria-label")?.endsWith("· Personal"));
    expect(group, "the account has its own picker group").toBeTruthy();
    await act(async () => group!.querySelector<HTMLButtonElement>('button[role="option"]')!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(picker.getAttribute("aria-label")).toContain("Personal");

    const creates = spyCreates();
    await type(composerField()!, "bill the other subscription");
    await act(async () => pressEnter(composerField()!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });
    expect(creates.configured.mock.calls[0][0].overrides.model).toMatchObject({ harness: "claude", account: "personal" });
    const result: Awaited<ReturnType<typeof bridgeApi.createConfiguredChat>> = await creates.configured.mock.results[0].value;
    expect(result.state.sessions.find(session => session.id === result.sessionId)?.account).toBe("personal");
    await act(async () => { await bridgeApi.resetHarnessConfig("claude"); });
  });
  it("drops the draft's account when a specialist is chosen, since it runs on your own login", async () => {
    const config = await bridgeApi.configState();
    const claude = config.harnesses.find(harness => harness.id === "claude")!;
    await act(async () => { await bridgeApi.saveHarnessConfig({ ...claude, advanced: { accounts: [{ id: "personal", label: "Personal", homePath: "~/.claude_personal" }] } }); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    const picker = () => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-label")?.startsWith("Chat model:"))!;
    await act(async () => picker().click());
    const group = [...document.querySelectorAll<HTMLElement>('[role="group"]')].find(item => item.getAttribute("aria-label")?.endsWith("· Personal"))!;
    await act(async () => group.querySelector<HTMLButtonElement>('button[role="option"]')!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(picker().getAttribute("aria-label")).toContain("Personal");

    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Agent: None"]')!.click());
    const specialist = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Agent"] [role="menuitemradio"]')].find(item => item.textContent !== "No agent")!;
    await act(async () => specialist.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
    expect(picker().getAttribute("aria-label")).not.toContain("Personal");

    const creates = spyCreates();
    await type(composerField()!, "look into the flaky test");
    await act(async () => pressEnter(composerField()!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });
    expect(creates.configured.mock.calls[0][0].overrides.model?.account).toBeUndefined();
    const result: Awaited<ReturnType<typeof bridgeApi.createConfiguredChat>> = await creates.configured.mock.results[0].value;
    expect(result.configuration.settings.model.account ?? null).toBeNull();
    expect(result.state.sessions.find(session => session.id === result.sessionId)?.account ?? null).toBeNull();
    await act(async () => { await bridgeApi.resetHarnessConfig("claude"); });
  });
  it("starts with saved project settings and keeps access edits local to the draft",async()=>{
    const state=await bridgeApi.state();const projectId=state.workspaces.find(w=>w.id==="demo-1")!.projectId!;
    const view=await bridgeApi.getChatDefaults(projectId);
    await bridgeApi.saveChatDefaults({projectId,expectedRevision:view.project?.revision??0,defaults:{target:{kind:"direct"},access:"ask",activeTurnInput:"queue"}});
    await act(async()=>byLabel("New Chat")!.click());
    expect(modeRadio("Direct")?.getAttribute("aria-checked")).toBe("true");
    const saveGlobal=vi.spyOn(bridgeApi,"savePermissionPolicy");
    const access=container.querySelector<HTMLButtonElement>('[aria-label="Access: User approval"]')!;
    await act(async()=>access.click());
    const full=[...document.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(b=>b.textContent==="Full access")!;
    await act(async()=>full.click());expect(saveGlobal).not.toHaveBeenCalled();
    const creates=spyCreates();await type(composerField()!,"project preference");await act(async()=>pressEnter(composerField()!));
    const result:Awaited<ReturnType<typeof bridgeApi.createConfiguredChat>>=await creates.configured.mock.results[0].value;
    expect(result.configuration.settings).toMatchObject({target:{kind:"direct"},access:"full",activeTurnInput:"queue"});
    expect((await bridgeApi.getChatDefaults(projectId)).project?.defaults.access).toBe("ask");
  });
  it("switches inherited project defaults without discarding the typed message",async()=>{
    const state=await bridgeApi.state();const first=state.workspaces.find(w=>w.id==="demo-1")!;const next=await bridgeApi.cloneWorkspaceRepo("owner/project-defaults-test");const other=next.workspaces.find(w=>!state.workspaces.some(old=>old.id===w.id))!;
    const view=await bridgeApi.getChatDefaults(other.projectId);
    await bridgeApi.saveChatDefaults({projectId:other.projectId,expectedRevision:view.project?.revision??0,defaults:{target:{kind:"direct"},createWorktree:false}});
    await act(async()=>byLabel("New Chat")!.click());await type(composerField()!,"keep these words");
    const repo=container.querySelector<HTMLButtonElement>('[aria-label="Chat context"] button')!;
    await act(async()=>repo.click());
    const option=[...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Repository"] [role="menuitemradio"]')].find(b=>b.textContent===other.title)!;
    await act(async()=>option.click());
    expect(composerField()!.value).toBe("keep these words");expect(modeRadio("Direct")?.getAttribute("aria-checked")).toBe("true");
    const creates=spyCreates();await act(async()=>pressEnter(composerField()!));expect(creates.configured.mock.calls[0][0].workspaceId).toBe(other.id);
  });
  it("refreshes stale defaults after rejecting the send and retains the message",async()=>{
    await act(async()=>byLabel("New Chat")!.click());await type(composerField()!,"retain on conflict");
    const state=await bridgeApi.state();const projectId=state.workspaces.find(w=>w.id==="demo-1")!.projectId!;const view=await bridgeApi.getChatDefaults(projectId);
    await bridgeApi.saveChatDefaults({projectId,expectedRevision:view.project?.revision??0,defaults:{target:{kind:"direct"}}});
    const creates=spyCreates();await act(async()=>pressEnter(composerField()!));
    expect(totalCreates(creates)).toBe(0);expect(composerField()!.value).toBe("retain on conflict");expect(modeRadio("Direct")?.getAttribute("aria-checked")).toBe("true");
  });

});
