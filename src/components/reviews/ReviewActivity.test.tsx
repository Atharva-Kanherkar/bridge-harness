// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { ReviewActivityEntry, ReviewPass } from "../../protocol/generated/protocol";
import { mergeActivity, ReviewActivity } from "./ReviewActivity";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let index = 0; index < 6; index += 1) await Promise.resolve(); };

const entry = (seq: number, kind: ReviewActivityEntry["kind"], over: Partial<ReviewActivityEntry> = {}): ReviewActivityEntry => ({
  jobId: "job-1", passId: "pass-claude", seq, kind, at: "2026-11-25T10:00:00.000Z", ...over,
});

const pass: ReviewPass = {
  id: "pass-claude", jobId: "job-1", slot: 0, attempt: 1, harness: "claude", model: "claude-opus-5-5", effort: "high", status: "running",
  deniedRequests: 0, tokensIn: 0, tokensOut: 0, costMicrousd: 0,
};

async function mount(entries: ReviewActivityEntry[], running = true) {
  const handlers: Array<(payload: { jobId: string; entry: ReviewActivityEntry }) => void> = [];
  vi.spyOn(bridgeApi, "onReviewActivity").mockImplementation(async handler => { handlers.push(handler); return () => {}; });
  const read = vi.spyOn(bridgeApi, "reviewActivity").mockResolvedValue({ entries });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(<ReviewActivity jobId="job-1" passes={[pass]} running={running} />); await flush(); });
  return { read, emit: (jobId: string, next: ReviewActivityEntry) => act(() => { handlers.forEach(handler => handler({ jobId, entry: next })); }) };
}

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.restoreAllMocks();
});

describe("mergeActivity", () => {
  it("keeps each entry once, by pass and sequence", () => {
    const merged = mergeActivity([entry(1, "prompt"), entry(2, "tool")], [entry(2, "tool"), entry(3, "message")]);
    expect(merged.map(item => item.seq)).toEqual([1, 2, 3]);
  });
});

describe("ReviewActivity", () => {
  it("shows a pass as a chat: Bridge's request folded, tools as steps, messages in full", async () => {
    await mount([
      entry(1, "prompt", { title: "Bridge asked for a review", text: "Review pull request #7: Cache it" }),
      entry(2, "tool", { title: "Read", text: "src/cache.rs" }),
      entry(3, "command", { title: "rg -n expire" }),
      entry(4, "denied", { title: "Refused a permission request", text: "Run cargo test" }),
      entry(5, "message", { text: "The cache never **expires**." }),
    ]);
    const thread = host!.querySelector<HTMLElement>('[aria-label="Claude activity"]')!;
    expect(thread.textContent).toContain("claude-opus-5-5 · high");
    expect(thread.textContent).toContain("2 steps");
    const request = [...thread.querySelectorAll("button")].find(button => button.textContent?.includes("Bridge asked for a review"))!;
    expect(request.getAttribute("aria-expanded")).toBe("false");
    expect(thread.textContent, "the request stays folded").not.toContain("Review pull request #7");
    act(() => request.click());
    expect(thread.textContent).toContain("Review pull request #7: Cache it");
    expect(thread.textContent).toContain("Readsrc/cache.rs");
    expect(thread.textContent).toContain("rg -n expire");
    expect(thread.textContent).toContain("Refused a permission requestRun cargo test");
    expect(thread.querySelector("strong")?.textContent, "messages render as Markdown").toBe("expires");
    expect(host!.querySelector('[role="status"]')!.textContent).toContain("Working…");
  });

  it("appends this job's live entries and ignores other jobs'", async () => {
    const { emit } = await mount([entry(1, "prompt", { title: "Bridge asked for a review", text: "go" })]);
    emit("job-1", entry(2, "tool", { title: "Grep", text: "unwrap" }));
    emit("job-2", entry(2, "tool", { jobId: "job-2", title: "Elsewhere" }));
    expect(host!.textContent).toContain("Grepunwrap");
    expect(host!.textContent).not.toContain("Elsewhere");
  });

  it("says what to expect before anything is recorded, and when nothing was", async () => {
    await mount([]);
    expect(host!.textContent).toContain("Nothing yet: the reviewer's messages and tool calls appear here as it works.");
    act(() => root?.unmount());
    host!.remove();
    await mount([], false);
    expect(host!.textContent).toContain("No activity was recorded for this review.");
    expect(host!.querySelector('[role="status"]')).toBeNull();
  });
});
