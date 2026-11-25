// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi, emptyReviewOverview } from "../../api";
import type { ReviewJob, ReviewOverview, ReviewOverviewFinding } from "../../protocol/generated/protocol";
import { keptRate, ReviewsScreen } from "./ReviewsScreen";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let index = 0; index < 8; index += 1) await Promise.resolve(); };

const job = (over: Partial<ReviewJob> = {}): ReviewJob => ({
  id: "job-1", workspaceId: "w", number: 7, headSha: "abc", status: "posted", trigger: "manual", postMode: "draft",
  harnesses: ["claude"], findingCounts: { total: 1, critical: 0, major: 1, minor: 0, nit: 0 },
  createdAt: "2026-11-24T10:00:00.000Z", startedAt: "2026-11-24T10:00:00.000Z",
  repoOwner: "octo", repoName: "api", prTitle: "Parse input", ...over,
});

const finding = (id: string, jobId: string, over: Partial<ReviewOverviewFinding["finding"]> = {}, title = "Parse input"): ReviewOverviewFinding => ({
  finding: {
    id, jobId, passId: "p1", path: "src/lib.rs", line: 4, side: "RIGHT", severity: "major", title: `Title ${id}`, body: "Body", confidence: 0.8,
    fingerprint: id, agreedBy: 1, outsideDiff: false, decision: "pending", ...over,
  },
  workspaceId: "w", repoOwner: "octo", repoName: "api", number: 7, prTitle: title, harnesses: ["claude"], createdAt: "2026-11-24T10:00:00.000Z",
});

function overview(over: Partial<ReviewOverview> = {}): ReviewOverview {
  const base = emptyReviewOverview();
  return {
    ...base,
    jobs: [
      job({ id: "draft", status: "drafted", number: 8, prTitle: "Add cache", harnesses: ["claude", "codex"], findingCounts: { total: 2, critical: 1, major: 0, minor: 1, nit: 0 } }),
      job({ id: "run", status: "reviewing", number: 9, prTitle: "Fix login" }),
      job({ id: "done", status: "posted" }),
      job({ id: "broken", status: "failed", number: 10, prTitle: "Bump deps", error: "Codex is signed out" }),
    ],
    stats: {
      ...base.stats, reviews: 4, running: 1, waiting: 1, posted: 1, failed: 1,
      findings: { total: 3, critical: 1, major: 1, minor: 1, nit: 0 }, kept: 2, edited: 1, dropped: 1, pending: 0, fixedLater: 1, agreed: 1,
      medianSeconds: 125, spendMicrousd: 1_250_000, tokensIn: 3_000_000, tokensOut: 9_000,
    },
    harnesses: [
      { harness: "claude", passes: 3, failed: 0, findings: 2, kept: 2, dropped: 0, medianSeconds: 110, spendMicrousd: 1_250_000 },
      { harness: "codex", passes: 2, failed: 1, findings: 1, kept: 0, dropped: 1, medianSeconds: 240, spendMicrousd: 0 },
    ],
    findings: [finding("f1", "draft", { severity: "critical", title: "Cache never expires" }, "Add cache"), finding("f2", "done", { title: "Unchecked unwrap", decision: "kept" })],
    repositories: ["octo/api", "octo/web"],
    ...over,
  };
}

async function mount(data: ReviewOverview | (() => ReviewOverview), props: { onOpenChat?: (sessionId: string) => void } = {}) {
  const read = vi.spyOn(bridgeApi, "reviewOverview").mockImplementation(async () => typeof data === "function" ? data() : data);
  const handlers: Array<(payload: { job: ReviewJob }) => void> = [];
  vi.spyOn(bridgeApi, "onReviewJobUpdated").mockImplementation(async handler => { handlers.push(handler); return () => {}; });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(<ReviewsScreen {...props} />); await flush(); });
  return { read, emit: (next: ReviewJob) => act(() => { handlers.forEach(handler => handler({ job: next })); }) };
}

const tile = (label: string) => host!.querySelector<HTMLElement>(`[data-tile="${label}"]`)!;
const radio = (group: string, label: string) =>
  [...host!.querySelectorAll<HTMLButtonElement>(`[role="radiogroup"][aria-label="${group}"] [role="radio"]`)].find(item => item.textContent?.startsWith(label))!;

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("keptRate", () => {
  it("is undefined until something is decided, then kept or edited over decided", () => {
    expect(keptRate({ kept: 0, edited: 0, dropped: 0 })).toBeUndefined();
    expect(keptRate({ kept: 2, edited: 1, dropped: 1 })).toBe(0.75);
  });
});

describe("ReviewsScreen", () => {
  it("shows the period's numbers in tiles", async () => {
    await mount(overview());
    expect(tile("Reviews").textContent).toContain("4");
    expect(tile("Reviews").textContent).toContain("1 posted · 1 waiting · 1 failed");
    expect(tile("Findings").textContent).toContain("1 critical");
    expect(tile("Findings").textContent).toContain("1 both reviewers flagged");
    expect(tile("Kept").textContent).toContain("75%");
    expect(tile("Kept").textContent).toContain("3 of 4 decided");
    expect(tile("Fixed later").textContent).toContain("1");
    expect(tile("Median time").textContent).toContain("2m 05s");
    expect(tile("Spend").textContent).toContain("$1.25");
    expect(tile("Spend").textContent, "Codex reports no cost").toContain("some reviewers report no cost");
  });

  it("says – rather than 0% when nothing was decided", async () => {
    const data = overview();
    data.stats = { ...data.stats, kept: 0, edited: 0, dropped: 0, pending: 3 };
    await mount(data);
    expect(tile("Kept").textContent).toContain("–");
    expect(tile("Kept").textContent).toContain("Nothing decided yet");
    expect(tile("Kept").textContent).not.toContain("0%");
  });

  it("lists drafts under Needs you, running jobs under Running, and compares reviewers", async () => {
    await mount(overview());
    const needs = host!.querySelector<HTMLElement>('[aria-label="Needs you"]')!;
    expect([...needs.querySelectorAll("[data-open-job]")].map(item => item.getAttribute("data-open-job"))).toEqual(["draft"]);
    expect(needs.textContent).toContain("Add cache");
    expect(needs.textContent).toContain("octo/api #8");
    const runningList = host!.querySelector<HTMLElement>('[aria-label="Running"]')!;
    expect([...runningList.querySelectorAll("[data-open-job]")].map(item => item.getAttribute("data-open-job"))).toEqual(["run"]);
    const codex = host!.querySelector<HTMLElement>('[data-reviewer="codex"]')!;
    expect(codex.textContent).toContain("Codex");
    expect(codex.textContent, "kept over decided: 0 of 1").toContain("0%");
    expect(codex.textContent).toContain("Not reported");
    expect(host!.querySelector('[data-reviewer="claude"]')!.textContent).toContain("100%");
  });

  it("opens a draft's triage from Needs you", async () => {
    await mount(overview());
    const detail = vi.spyOn(bridgeApi, "getReview").mockImplementation(async id => ({
      job: overview().jobs.find(item => item.id === id)!, passes: [], findings: [], summary: "**Claude:** Looks risky.",
    }));
    await act(async () => { host!.querySelector<HTMLButtonElement>('[data-open-job="draft"]')!.click(); await flush(); });
    expect(detail).toHaveBeenCalledWith("draft");
    const sheet = host!.querySelector<HTMLElement>('[data-review-detail="draft"]')!;
    expect(sheet.getAttribute("role")).toBe("dialog");
    expect(sheet.textContent).toContain("Add cache");
    expect(sheet.textContent).toContain("Looks risky.");
    expect(sheet.querySelector('[aria-label="Triage"]'), "a draft opens the pane's triage").not.toBeNull();
    await act(async () => { sheet.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await flush(); });
    expect(host!.querySelector("[data-review-detail]")).toBeNull();
  });

  it("narrows the reviews by status and the findings by text and severity", async () => {
    await mount(overview());
    const rows = () => [...host!.querySelectorAll("[data-job-row]")].map(item => item.getAttribute("data-job-row"));
    expect(rows()).toEqual(["draft", "run", "done", "broken"]);
    act(() => radio("Show reviews", "Failed").click());
    expect(rows()).toEqual(["broken"]);
    act(() => radio("Show reviews", "Needs you").click());
    expect(rows()).toEqual(["draft"]);

    act(() => radio("History", "Findings").click());
    const findingRows = () => [...host!.querySelectorAll("[data-finding-row]")].map(item => item.getAttribute("data-finding-row"));
    expect(findingRows()).toEqual(["f1", "f2"]);
    const search = host!.querySelector<HTMLInputElement>('input[aria-label="Search findings"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(search, "unwrap");
      search.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(findingRows()).toEqual(["f2"]);
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(search, "");
      search.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    act(() => radio("Severity", "Critical").click());
    expect(findingRows()).toEqual(["f1"]);
  });

  it("rereads once for a burst of job updates, and on a filter change", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { read, emit } = await mount(overview());
    expect(read).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenLastCalledWith(30, undefined);
    emit(job({ id: "run", status: "reviewing" }));
    emit(job({ id: "run", status: "drafted" }));
    await act(async () => { vi.advanceTimersByTime(500); await flush(); });
    expect(read).toHaveBeenCalledTimes(2);
    await act(async () => { radio("Period", "All").click(); await flush(); });
    expect(read).toHaveBeenLastCalledWith(undefined, undefined);
    const select = host!.querySelector<HTMLSelectElement>('select[aria-label="Repository"]')!;
    expect([...select.options].map(option => option.value)).toEqual(["", "octo/api", "octo/web"]);
    await act(async () => { select.value = "octo/web"; select.dispatchEvent(new Event("change", { bubbles: true })); await flush(); });
    expect(read).toHaveBeenLastCalledWith(undefined, "octo/web");
  });

  it("explains what to do when there are no reviews", async () => {
    await mount(emptyReviewOverview());
    expect(host!.textContent).toContain("No drafts are waiting");
    expect(host!.textContent).toContain("Nothing is running.");
    expect(host!.textContent).toContain("No reviewer has run in this period.");
    expect(host!.textContent).toContain("Start one from a pull request's Review button in the GitHub pane.");
    expect(tile("Median time").textContent).toContain("–");
  });
});

describe("ReviewsScreen: activity, handoff and settings", () => {
  const detailOf = (id: string) => async () => ({ job: overview().jobs.find(item => item.id === id)!, passes: [], findings: [] });

  it("opens a running review on what it is doing, and a draft on what it found", async () => {
    await mount(overview());
    vi.spyOn(bridgeApi, "getReview").mockImplementation(async id => detailOf(id)());
    vi.spyOn(bridgeApi, "reviewActivity").mockResolvedValue({ entries: [{ jobId: "run", passId: "p", seq: 1, kind: "tool", title: "Read", text: "src/login.rs", at: "2026-11-24T10:00:00.000Z" }] });
    vi.spyOn(bridgeApi, "onReviewActivity").mockResolvedValue(() => {});
    await act(async () => { host!.querySelector<HTMLButtonElement>('[data-open-job="run"]')!.click(); await flush(); });
    const view = () => host!.querySelector<HTMLElement>('[role="radiogroup"][aria-label="Review view"] [aria-checked="true"]')!.textContent;
    expect(view()).toBe("Activity");
    expect(host!.querySelector('[data-review-activity="run"]')!.textContent).toContain("Readsrc/login.rs");
    await act(async () => { host!.querySelector<HTMLElement>('[data-review-detail]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await flush(); });
    await act(async () => { host!.querySelector<HTMLButtonElement>('[data-open-job="draft"]')!.click(); await flush(); });
    expect(view()).toBe("Findings");
  });

  it("names the chat a review went to and opens it, or says why it did not go", async () => {
    const onOpenChat = vi.fn();
    const data = overview();
    data.jobs = data.jobs.map(item => item.id === "done" ? { ...item, handoffSessionId: "chat-7", handoffChat: "Add the cache", handoffAt: "2026-11-24T10:05:00.000Z" }
      : item.id === "draft" ? { ...item, handoffNote: "Not sent: no chat in this project works on the branch feat/x." } : item);
    await mount(data, { onOpenChat });
    vi.spyOn(bridgeApi, "getReview").mockImplementation(async id => ({ job: data.jobs.find(item => item.id === id)!, passes: [], findings: [] }));
    await act(async () => { host!.querySelector<HTMLElement>('[data-job-row="done"]')!.click(); await flush(); });
    const sent = host!.querySelector<HTMLElement>("[data-handoff]")!;
    expect(sent.textContent).toContain("Sent to");
    await act(async () => { [...sent.querySelectorAll("button")].find(button => button.textContent === "Add the cache")!.click(); await flush(); });
    expect(onOpenChat).toHaveBeenCalledWith("chat-7");
    expect(host!.querySelector("[data-review-detail]"), "opening the chat closes the review").toBeNull();
    await act(async () => { host!.querySelector<HTMLElement>('[data-job-row="draft"]')!.click(); await flush(); });
    expect(host!.querySelector("[data-handoff]")!.textContent).toBe("Not sent: no chat in this project works on the branch feat/x.");
  });

  it("holds each reviewer's model and effort in its Settings, saved from there", async () => {
    await mount(overview());
    vi.spyOn(bridgeApi, "health").mockResolvedValue({ adapters: [{ id: "claude", label: "Claude", available: true, models: [{ id: "claude-opus-5-5", label: "Opus 5.5" }] }] } as never);
    vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({ settings: { harnesses: {}, systemPrompt: "" }, defaultSystemPrompt: "default" });
    vi.spyOn(bridgeApi, "getReviewJobSettings").mockResolvedValue({});
    const save = vi.spyOn(bridgeApi, "saveReviewerSettings").mockImplementation(async settings => ({ settings, defaultSystemPrompt: "default" }));
    await act(async () => { radio("Reviews view", "Settings").click(); await flush(); });
    expect(host!.querySelector('[data-tile="Reviews"]'), "the overview steps aside").toBeNull();
    const model = host!.querySelector<HTMLSelectElement>('select[aria-label="Claude reviewer model"], [aria-label="Claude reviewer model"]');
    expect(model, "Claude's model is set here").not.toBeNull();
    expect(host!.textContent).toContain("Every review on that harness uses them");
    expect(save).not.toHaveBeenCalled();
  });
});
