// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { ReviewFinding, ReviewFindingDecisionInput, ReviewJob } from "../../protocol/generated/protocol";
import { groupFindings, ReviewDraft } from "./ReviewDraft";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let index = 0; index < 5; index += 1) await Promise.resolve(); };

const job: ReviewJob = {
  id: "job-1", workspaceId: "w", number: 7, headSha: "abc", status: "drafted", trigger: "manual", postMode: "draft",
  harnesses: ["claude", "codex"], findingCounts: { total: 4, critical: 1, major: 1, minor: 1, nit: 1 }, createdAt: "2026-11-24T10:00:00.000Z",
};

const finding = (id: string, over: Partial<ReviewFinding> = {}): ReviewFinding => ({
  id, jobId: "job-1", passId: "pass-claude", path: "src/b.rs", line: 10, side: "RIGHT", severity: "minor",
  title: `Title ${id}`, body: `Body ${id}`, confidence: 0.5, fingerprint: id, agreedBy: 1, outsideDiff: false, decision: "pending", ...over,
});

const FINDINGS = [
  finding("b-minor"),
  finding("b-critical", { severity: "critical", confidence: 0.9 }),
  finding("a-nit", { path: "src/a.rs", severity: "nit", startLine: 3, line: 6, passId: "pass-codex", agreedBy: 2 }),
  finding("outside", { path: "src/z.rs", severity: "major", line: 400, outsideDiff: true }),
];

/** A store that applies decisions the way `reviews/decide_review` does. */
function mockStore(initial: ReviewFinding[]) {
  let findings = initial.map(item => ({ ...item }));
  vi.spyOn(bridgeApi, "getReview").mockImplementation(async () => ({
    job, findings,
    passes: [
      { id: "pass-claude", jobId: "job-1", slot: 0, attempt: 1, harness: "claude", status: "done", deniedRequests: 0, tokensIn: 1, tokensOut: 1, costMicrousd: 0 },
      { id: "pass-codex", jobId: "job-1", slot: 1, attempt: 1, harness: "codex", status: "done", deniedRequests: 0, tokensIn: 1, tokensOut: 1, costMicrousd: 0 },
    ],
  }));
  const decide = vi.spyOn(bridgeApi, "decideReview").mockImplementation(async (_jobId, decisions: ReviewFindingDecisionInput[]) => {
    findings = findings.map(item => {
      const decision = decisions.find(entry => entry.findingId === item.id);
      return decision ? { ...item, decision: decision.decision, editedBody: decision.decision === "edited" ? decision.editedBody : null } : item;
    });
    return { findings };
  });
  const post = vi.spyOn(bridgeApi, "postReview").mockResolvedValue({ job: { ...job, status: "posting" } });
  return { decide, post };
}

async function mount(onPosting = vi.fn()) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(<ReviewDraft job={job} onPosting={onPosting} />); await flush(); });
  return onPosting;
}

const item = (id: string) => host!.querySelector<HTMLElement>(`[data-finding="${id}"]`)!;
const button = (scope: ParentNode, name: string) => [...scope.querySelectorAll("button")].find(candidate => candidate.textContent?.trim().startsWith(name))!;
const postButton = () => [...host!.querySelectorAll("button")].find(candidate => candidate.textContent?.includes("Post"))!;
const press = async (target: HTMLElement, key: string) => {
  await act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })); await flush(); });
};

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.restoreAllMocks();
});

describe("ReviewDraft", () => {
  it("groups by file, outside the diff last, and orders by severity then confidence", () => {
    const groups = groupFindings(FINDINGS);
    expect(groups.map(group => group.label)).toEqual(["src/a.rs", "src/b.rs", "Outside the diff"]);
    expect(groups[1].findings.map(item => item.id)).toEqual(["b-critical", "b-minor"]);
  });

  it("shows where, how sure, which reviewer, and when both agreed", async () => {
    mockStore(FINDINGS);
    await mount();
    const nit = item("a-nit");
    expect(nit.textContent).toContain("src/a.rs:3–6");
    expect(nit.textContent).toContain("50% sure");
    expect(nit.textContent).toContain("Codex");
    expect(nit.textContent).toContain("Both reviewers");
    expect(item("b-minor").textContent).not.toContain("Both reviewers");
    expect(item("b-minor").textContent).toContain("Claude");
  });

  it("keeps, drops and edits, and posts with the count", async () => {
    const { decide, post } = mockStore(FINDINGS);
    const onPosting = await mount();
    expect(postButton().disabled).toBe(true);
    expect(postButton().textContent?.trim()).toBe("Post");
    expect(host!.textContent).toContain("Keep or edit at least one finding to post.");

    await act(async () => { button(item("b-critical"), "Keep").click(); await flush(); });
    expect(decide).toHaveBeenLastCalledWith("job-1", [{ findingId: "b-critical", decision: "kept", editedBody: null }]);
    expect(item("b-critical").getAttribute("data-decision")).toBe("kept");
    expect(postButton().textContent).toContain("Post 1 finding");

    await act(async () => { button(item("b-minor"), "Edit").click(); await flush(); });
    const editor = item("b-minor").querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor, "Say it plainly.");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    await act(async () => { button(item("b-minor"), "Save edit").click(); await flush(); });
    expect(decide).toHaveBeenLastCalledWith("job-1", [{ findingId: "b-minor", decision: "edited", editedBody: "Say it plainly." }]);
    expect(item("b-minor").textContent).toContain("Say it plainly.");
    expect(item("b-minor").textContent).toContain("Edited");

    await act(async () => { button(item("outside"), "Drop").click(); await flush(); });
    expect(item("outside").getAttribute("data-decision")).toBe("dropped");
    expect(postButton().textContent).toContain("Post 2 findings");

    await act(async () => { postButton().click(); await flush(); });
    expect(post).toHaveBeenCalledWith("job-1");
    expect(onPosting).toHaveBeenCalledWith(expect.objectContaining({ status: "posting" }));
  });

  it("never posts past an unsaved edit, and deciding another finding keeps it", async () => {
    const { decide, post } = mockStore(FINDINGS);
    await mount();
    await act(async () => { button(host!, "Keep all").click(); await flush(); });
    await act(async () => { button(item("b-minor"), "Edit").click(); await flush(); });
    const editor = () => item("b-minor").querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor(), "Better words.");
      editor().dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(postButton().disabled, "the screen and GitHub would disagree").toBe(true);
    expect(host!.textContent).toContain("Save or cancel your edit to post.");

    await act(async () => { button(item("b-critical"), "Drop").click(); await flush(); });
    expect(editor().value, "another decision leaves the open edit").toBe("Better words.");
    await act(async () => { button(host!, "Keep all").click(); await flush(); });
    expect(decide.mock.lastCall![1].map(entry => entry.findingId), "Keep all passes over the finding being edited").not.toContain("b-minor");
    await press(item("b-minor"), "d");
    expect(editor().value, "K and D wait while the finding is being edited").toBe("Better words.");
    expect(postButton().disabled).toBe(true);

    await act(async () => { button(item("b-minor"), "Save edit").click(); await flush(); });
    expect(decide).toHaveBeenLastCalledWith("job-1", [{ findingId: "b-minor", decision: "edited", editedBody: "Better words." }]);
    expect(postButton().disabled).toBe(false);
    await act(async () => { postButton().click(); await flush(); });
    expect(post).toHaveBeenCalledWith("job-1");
  });

  it("names the findings still undecided beside Post", async () => {
    mockStore(FINDINGS);
    await mount();
    await act(async () => { button(item("b-critical"), "Keep").click(); await flush(); });
    await act(async () => { button(item("outside"), "Drop").click(); await flush(); });
    expect(host!.textContent).toContain("1 kept, 2 not decided");
  });

  it("keeps all and drops all in one decision", async () => {
    const { decide } = mockStore(FINDINGS);
    await mount();
    await act(async () => { button(host!, "Keep all").click(); await flush(); });
    expect(decide).toHaveBeenLastCalledWith("job-1", FINDINGS.map(item => ({ findingId: item.id, decision: "kept" })));
    expect(postButton().textContent).toContain("Post 4 findings");
    await act(async () => { button(host!, "Drop all").click(); await flush(); });
    expect(postButton().disabled).toBe(true);
  });

  it("is driven from the keyboard: arrows move, K keeps, D drops, E edits", async () => {
    const { decide } = mockStore(FINDINGS);
    await mount();
    const first = item("a-nit");
    first.focus();
    await press(first, "k");
    expect(decide).toHaveBeenLastCalledWith("job-1", [{ findingId: "a-nit", decision: "kept", editedBody: null }]);
    await press(item("a-nit"), "ArrowDown");
    expect(document.activeElement).toBe(item("b-critical"));
    await press(item("b-critical"), "d");
    expect(item("b-critical").getAttribute("data-decision")).toBe("dropped");
    await press(item("b-critical"), "ArrowUp");
    expect(document.activeElement).toBe(item("a-nit"));
    await press(item("a-nit"), "e");
    expect(item("a-nit").querySelector("textarea")).not.toBeNull();
    // Typing in the editor is text, not a shortcut.
    const editor = item("a-nit").querySelector("textarea")!;
    const calls = decide.mock.calls.length;
    await press(editor, "d");
    expect(decide.mock.calls.length).toBe(calls);
    await press(editor, "Escape");
    expect(item("a-nit").querySelector("textarea")).toBeNull();
  });

  it("posts a summary-only review when there are no findings, and shows a refused post", async () => {
    const { post } = mockStore([]);
    post.mockRejectedValueOnce(new Error("Only a drafted review waiting for you can be posted; this one is posting"));
    await mount();
    expect(host!.textContent).toContain("found nothing to flag");
    expect(postButton().textContent).toContain("Post summary");
    expect(postButton().disabled).toBe(false);
    await act(async () => { postButton().click(); await flush(); });
    expect(host!.querySelector('[role="alert"]')!.textContent).toContain("this one is posting");
  });
});
