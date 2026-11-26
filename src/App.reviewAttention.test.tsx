// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import type { ReviewJob } from "./protocol/generated/protocol";
import type { BridgeState } from "./types";

const notifyAttention = vi.hoisted(() => vi.fn());
vi.mock("./attention", () => ({ notifyAttention }));

const wait = (ms: number) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });

const job = (over: Partial<ReviewJob> = {}): ReviewJob => ({
  id: "j7", workspaceId: "w", number: 7, headSha: "abc", status: "queued", trigger: "manual", postMode: "draft",
  harnesses: ["claude"], findingCounts: { total: 2, critical: 0, major: 2, minor: 0, nit: 0 }, createdAt: "2026-11-24T10:00:00.000Z", ...over,
});

const state: BridgeState = {
  projects: [{ id: "p", name: "Demo", path: "/repo" } as BridgeState["projects"][number]],
  workspaces: [{ id: "w", projectId: "p", city: "Oslo", title: "Demo", branch: "main", path: "/repo", status: "ready" } as BridgeState["workspaces"][number]],
  sessions: [],
  events: [],
};

describe("App review attention", () => {
  let container: HTMLDivElement;
  let root: Root;
  let emit: (job: ReviewJob) => Promise<void>;
  let AppComponent: typeof import("./App")["App"];

  beforeEach(async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => { store.set(key, String(value)); }, removeItem: (key: string) => { store.delete(key); }, clear: () => store.clear(), key: () => null, length: 0 },
    });
    notifyAttention.mockReset();
    vi.spyOn(bridgeApi, "state").mockResolvedValue(state);
    const handlers: Array<(payload: { job: ReviewJob }) => void> = [];
    vi.spyOn(bridgeApi, "onReviewJobUpdated").mockImplementation(async handler => {
      handlers.push(handler);
      return () => { handlers.splice(handlers.indexOf(handler), 1); };
    });
    emit = async next => { await act(async () => { handlers.forEach(handler => handler({ job: next })); }); };
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    ({ App: AppComponent } = await import("./App"));
    await act(async () => { root.render(<AppComponent />); });
    await wait(40);
  }, 30_000);

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  const toast = (headline: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${headline} — open the pull request"]`);

  it("raises one toast when a draft is ready, one when it is posted, and one on failure", async () => {
    for (const status of ["queued", "preparing", "reviewing", "drafted"] as const) await emit(job({ status }));
    expect(toast("Review ready on PR #7")).not.toBeNull();
    expect(notifyAttention).toHaveBeenCalledWith("Review ready on PR #7", "2 findings to triage from Claude");
    // A decision republishes the draft: not news.
    await emit(job({ status: "drafted" }));
    expect(notifyAttention).toHaveBeenCalledTimes(1);

    await emit(job({ status: "posting" }));
    await emit(job({ status: "posted", postedReviewUrl: "https://github.com/o/r/pull/7#pullrequestreview-1" }));
    expect(toast("Review posted on PR #7")).not.toBeNull();

    await emit(job({ id: "j9", number: 9, status: "reviewing" }));
    await emit(job({ id: "j9", number: 9, status: "failed", error: "claude could not review: signed out" }));
    expect(toast("Review failed on PR #9")!.textContent).toContain("Claude could not review.");
    expect(notifyAttention).toHaveBeenCalledTimes(3);
  });

  it("raises a failed post again when the retry fails too", async () => {
    await emit(job({ status: "reviewing" }));
    await emit(job({ status: "drafted" }));
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await emit(job({ status: "posting" }));
      await emit(job({ status: "drafted", error: `GitHub refused (${attempt})` }));
    }
    const failed = notifyAttention.mock.calls.filter(([headline]) => headline === "Posting the review on PR #7 failed");
    expect(failed).toHaveLength(2);
    expect(container.querySelectorAll('button[aria-label="Posting the review on PR #7 failed — open the pull request"]')).toHaveLength(1);
    expect(toast("Posting the review on PR #7 failed")!.textContent, "the card shows the newest reason").toContain("GitHub refused (1)");
  });

  it("says nothing for a job it first sees already drafted", async () => {
    await emit(job({ status: "drafted" }));
    expect(toast("Review ready on PR #7")).toBeNull();
    expect(notifyAttention).not.toHaveBeenCalled();
  });

  it("opening the toast takes it away, and says so when its workspace has no chat to open", async () => {
    await emit(job({ status: "reviewing" }));
    await emit(job({ status: "failed", error: "boom" }));
    const card = toast("Review failed on PR #7")!;
    await act(async () => { card.click(); });
    await wait(20);
    expect(toast("Review failed on PR #7")).toBeNull();
    expect(container.textContent).toContain("Review failed on PR #7, but its workspace has no open chat to show it in.");
  });

  it("opening the toast opens the pull request in the GitHub pane", async () => {
    await act(async () => root.unmount());
    vi.spyOn(bridgeApi, "state").mockResolvedValue({
      ...state,
      sessions: [{ id: "chat", workspaceId: "w", kind: "chat", label: "Chat", title: "Chat", status: "ready", harness: "claude", continuationFidelity: "native", metricSource: "provider", restorationMode: "fresh" } as BridgeState["sessions"][number]],
    });
    const status = vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "available" }, repository: { host: "github.com", owner: "o", name: "r" } });
    const pullRequest = vi.spyOn(bridgeApi, "githubPullRequest").mockRejectedValue(new Error("offline"));
    root = createRoot(container);
    await act(async () => { root.render(<AppComponent />); });
    await wait(40);
    await emit(job({ status: "reviewing" }));
    await emit(job({ status: "drafted" }));
    await act(async () => { toast("Review ready on PR #7")!.click(); });
    await wait(60);
    expect(toast("Review ready on PR #7")).toBeNull();
    expect(status).toHaveBeenCalled();
    expect(pullRequest).toHaveBeenCalledWith("w", 7);
  });
});
