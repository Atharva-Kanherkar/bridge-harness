// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { MemoryRecord } from "../protocol/generated/protocol";
import { TasteRules } from "./TasteRules";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let index = 0; index < 6; index += 1) await Promise.resolve(); };

const rule = (id: string, over: Partial<MemoryRecord>): MemoryRecord => ({
  id, scopeKey: "project:octo/repo", kind: "review_avoid", body: `Rule ${id}`, provenance: "model_proposal", status: "active",
  validFrom: "2026-11-01T00:00:00Z", createdAt: "2026-11-01T00:00:00Z", updatedAt: "2026-11-01T00:00:00Z", layer: "taste", ...over,
});

function mockRules() {
  let proposed = [rule("learned", { status: "proposed", body: "Don't ask for a doc comment on `helper`.", evidenceCount: 5, rationale: "Dropped on every PR." })];
  let active = [
    rule("mine", { scopeKey: "account:local", kind: "code_prefer", body: "No `unwrap()` outside tests", provenance: "user_explicit" }),
    rule("repo", { scopeKey: "project:octo/repo", kind: "review_prefer", body: "Flag bounds checks that use `<=`", evidenceCount: 3, pathGlobs: ["src/**"] }),
  ];
  let paused: MemoryRecord[] = [];
  vi.spyOn(bridgeApi, "listTasteRules").mockImplementation(async status => ({
    rules: status === "proposed" ? proposed : status === "paused" ? paused : active,
    neverUsed: status === "active" ? ["repo"] : [],
  }));
  const state = vi.spyOn(bridgeApi, "setTasteRuleState").mockImplementation(async (id, change) => {
    if (change.state === "paused") {
      const moved = active.find(item => item.id === id)!;
      active = active.filter(item => item.id !== id);
      paused = [{ ...moved, status: "paused", pausedReason: "Paused by you." }, ...paused];
    }
    if (change.state === "active") {
      const moved = paused.find(item => item.id === id)!;
      paused = paused.filter(item => item.id !== id);
      active = [{ ...moved, status: "active", pausedReason: undefined }, ...active];
    }
    if (change.pinned !== undefined) active = active.map(item => item.id === id ? { ...item, pinned: change.pinned } : item);
    return rule(id, {});
  });
  const supersede = vi.spyOn(bridgeApi, "supersedeMemoryRecord").mockImplementation(async (id, body) => {
    active = active.map(item => item.id === id ? { ...item, body } : item);
    return rule(id, { body });
  });
  vi.spyOn(bridgeApi, "getTasteEvidence").mockResolvedValue({
    signals: [
      { id: "s1", at: "2026-10-01T00:00:00Z", source: "review_decision", verdict: "kept", title: "Bounds check uses <=", path: "src/get.rs", prNumber: 101, weight: 1 },
      { id: "s2", at: "2026-10-02T00:00:00Z", source: "review_fixed", verdict: "fixed", title: "Bounds check uses <=", path: "src/get.rs", prNumber: 102, weight: 1 },
    ],
    swept: 1,
  });
  const review = vi.spyOn(bridgeApi, "reviewMemoryRecords").mockImplementation(async (approve, _reject, scopeKey) => {
    const moved = proposed.filter(item => approve.includes(item.id));
    proposed = proposed.filter(item => !approve.includes(item.id) && !_reject.includes(item.id));
    active = [...moved.map(item => ({ ...item, status: "active" })), ...active];
    return { scopeKey: scopeKey ?? "account:local", approved: approve, rejected: _reject, refused: [] };
  });
  const add = vi.spyOn(bridgeApi, "addTasteRule").mockImplementation(async params => rule("new", { ...params, provenance: "user_explicit" }));
  const remove = vi.spyOn(bridgeApi, "deleteMemoryRecord").mockImplementation(async id => {
    active = active.filter(item => item.id !== id);
    return rule(id, { status: "deleted" });
  });
  return { review, add, remove, state, supersede };
}

async function mount(onError = vi.fn(), onCount = vi.fn()) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(<TasteRules onError={onError} onCount={onCount} />); await flush(); });
  return { onError, onCount };
}

const button = (scope: ParentNode, name: string) => [...scope.querySelectorAll("button")].find(candidate => candidate.textContent?.trim().startsWith(name) || candidate.getAttribute("aria-label")?.startsWith(name))!;
const row = (id: string) => host!.querySelector<HTMLElement>(`[data-taste-rule="${id}"]`)!;
const type = async (element: HTMLInputElement | HTMLTextAreaElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), "value")!.set!;
  await act(async () => { setter.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); await flush(); });
};

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.restoreAllMocks();
});

describe("TasteRules", () => {
  it("lists learned rules waiting for you, with where they came from, and approves one", async () => {
    const { review } = mockRules();
    const { onCount } = await mount();
    expect(onCount).toHaveBeenLastCalledWith(1);
    const waiting = host!.querySelector('[aria-label="Taste rules waiting for you"]')!;
    expect(waiting.textContent).toContain("Review · don't raise");
    expect(waiting.textContent).toContain("Don't ask for a doc comment on `helper`.");
    expect(waiting.textContent).toContain("octo/repo");
    expect(waiting.textContent).toContain("Dropped on every PR.");
    await act(async () => { button(row("learned"), "Approve").click(); await flush(); });
    expect(review).toHaveBeenCalledWith(["learned"], [], "project:octo/repo");
    expect(host!.querySelector('[aria-label="Taste rules waiting for you"]')).toBeNull();
    expect(onCount).toHaveBeenLastCalledWith(0);
    expect(host!.querySelector('[aria-label="Taste rules for octo/repo"]')!.textContent).toContain("doc comment");
  });

  it("groups active rules by repository, every repository last, and opens the decisions behind one", async () => {
    mockRules();
    await mount();
    const groups = [...host!.querySelectorAll('ul[aria-label^="Taste rules for"]')].map(list => list.getAttribute("aria-label"));
    expect(groups).toEqual(["Taste rules for octo/repo", "Taste rules for Every repository"]);
    expect(row("mine").textContent).toContain("Written by you");
    expect(row("repo").textContent).toContain("src/**");
    await act(async () => { button(row("repo"), "From 3 decisions").click(); await flush(); });
    const decisions = row("repo").querySelector('[aria-label="Decisions behind this rule"]')!;
    expect(decisions.textContent).toContain("Kept");
    expect(decisions.textContent).toContain("Fixed");
    expect(decisions.textContent).toContain("#102");
    expect(decisions.textContent).toContain("1 older decision no longer kept");
  });

  it("writes a rule for a repository or for every repository, and deletes one", async () => {
    const { add, remove } = mockRules();
    await mount();
    await act(async () => { button(host!, "New rule").click(); await flush(); });
    await act(async () => { button(host!, "Code · write it this way").click(); await flush(); });
    await type(host!.querySelector<HTMLTextAreaElement>('[aria-label="Rule"]')!, "Prefer early returns");
    await type(host!.querySelector<HTMLInputElement>('[aria-label="Repository"]')!, "octo/repo");
    await type(host!.querySelector<HTMLInputElement>('[aria-label="Paths"]')!, "src/**, , tests/**");
    await act(async () => { button(host!, "Save rule").click(); await flush(); });
    expect(add).toHaveBeenCalledWith({ scopeKey: "project:octo/repo", kind: "code_prefer", body: "Prefer early returns", pathGlobs: ["src/**", "tests/**"] });

    // The repository and kind stay for the next rule; this one is for every repository.
    await act(async () => { button(host!, "New rule").click(); await flush(); });
    expect(host!.querySelector<HTMLInputElement>('[aria-label="Repository"]')!.value).toBe("octo/repo");
    await act(async () => { button(host!, "Review · don't raise").click(); await flush(); });
    await type(host!.querySelector<HTMLInputElement>('[aria-label="Repository"]')!, "");
    await type(host!.querySelector<HTMLTextAreaElement>('[aria-label="Rule"]')!, "Skip import-order nits");
    await act(async () => { button(host!, "Save rule").click(); await flush(); });
    expect(add).toHaveBeenLastCalledWith(expect.objectContaining({ scopeKey: "account:local", kind: "review_avoid" }));

    await act(async () => { button(row("mine"), "Delete rule").click(); await flush(); });
    expect(remove).toHaveBeenCalledWith("mine");
    expect(row("mine")).toBeNull();
  });

  it("shows trust and rules never used, and pauses, resumes, pins and edits", async () => {
    const { state, supersede } = mockRules();
    await mount();
    expect(row("repo").textContent).toContain("Trust 0.0");
    expect(row("repo").textContent).toContain("Never used");
    expect(row("mine").textContent).not.toContain("Trust");
    expect(button(row("mine"), "Pin rule")).toBeUndefined();

    await act(async () => { button(row("repo"), "Pin rule").click(); await flush(); });
    expect(state).toHaveBeenLastCalledWith("repo", { pinned: true });
    expect(row("repo").textContent).toContain("Learned · pinned");

    await act(async () => { button(row("repo"), "Edit rule").click(); await flush(); });
    await type(row("repo").querySelector<HTMLTextAreaElement>("textarea")!, "Flag bounds checks that use `<=` on a length");
    await act(async () => { button(row("repo"), "Save").click(); await flush(); });
    expect(supersede).toHaveBeenCalledWith("repo", "Flag bounds checks that use `<=` on a length");

    await act(async () => { button(row("repo"), "Pause rule").click(); await flush(); });
    expect(state).toHaveBeenLastCalledWith("repo", { state: "paused" });
    const resting = host!.querySelector('[aria-label="Paused taste rules"]')!;
    expect(resting.textContent).toContain("Paused by you.");
    await act(async () => { button(resting, "Resume").click(); await flush(); });
    expect(state).toHaveBeenLastCalledWith("repo", { state: "active" });
    expect(host!.querySelector('[aria-label="Paused taste rules"]')).toBeNull();
  });
});
