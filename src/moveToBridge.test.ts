import { describe, expect, it } from "vitest";
import type { DetectedImportSource, ImportJob } from "./protocol/generated/protocol";
import { finishedLabel, folderLabel, importNotice, importPercent, newerJob, sourceSummary } from "./moveToBridge";

const source = (counts: Partial<DetectedImportSource["counts"]>, installed = true): DetectedImportSource => ({
  tool: "claude_code", label: "Claude Code", installed, supported: true, roots: [], note: null,
  counts: { chats: 0, projects: 0, workspaces: 0, memories: 0, setup: 0, background: 0, ...counts },
});

const job = (fields: Partial<ImportJob>): ImportJob => ({
  jobId: "j1", tool: "claude_code", status: "running", total: 0, processed: 0, imported: 0,
  alreadyInBridge: 0, changed: 0, failed: 0, leftOut: 0, notCarried: [], errors: [],
  startedAt: "2026-11-19T10:00:00Z", updatedAt: "2026-11-19T10:00:00Z", ...fields,
});

describe("Move to Bridge logic", () => {
  it("summarizes what a tool holds, and says when it is absent or empty", () => {
    expect(sourceSummary(source({ chats: 1312, projects: 14, memories: 3, setup: 1 }))).toBe("1,312 chats in 14 projects · 1 setup item");
    expect(sourceSummary(source({ chats: 1, projects: 1 }))).toBe("1 chat in 1 project");
    expect(sourceSummary(source({ chats: 2, projects: 1, background: 49798 }))).toBe("2 chats in 1 project · 49,798 background runs left out");
    expect(sourceSummary(source({ projects: 2, workspaces: 5 }))).toBe("5 workspaces in 2 repos");
    expect(sourceSummary(source({}))).toBe("Found, but nothing to bring over yet");
    expect(sourceSummary(source({ chats: 9 }, false))).toBe("Not found on this Mac");
  });

  it("shortens folders to their last two segments", () => {
    expect(folderLabel("/Users/you/work/invoice-app")).toBe("…/work/invoice-app");
    expect(folderLabel("/tmp")).toBe("/tmp");
    expect(folderLabel(null)).toBe("Folder unknown");
  });

  it("gives whole percent of the files read, and 0 before they are counted", () => {
    expect(importPercent(job({ total: 0, processed: 0 }))).toBe(0);
    expect(importPercent(job({ total: 33019, processed: 1240 }))).toBe(3);
    expect(importPercent(job({ total: 3, processed: 2 }))).toBe(66);
    expect(importPercent(job({ total: 4, processed: 4 }))).toBe(100);
  });

  it("never lets an older report of the same job replace a newer one", () => {
    const early = job({ processed: 25, updatedAt: "2026-11-19T10:00:05Z" });
    const later = job({ processed: 50, updatedAt: "2026-11-19T10:00:09Z" });
    expect(newerJob(later, early)).toBe(later);
    expect(newerJob(early, later)).toBe(later);
    expect(newerJob(null, early)).toBe(early);
    const other = job({ jobId: "j2", updatedAt: "2026-11-19T09:00:00Z" });
    expect(newerJob(later, other)).toBe(other);
  });

  it("labels a finished import with when it ended", () => {
    expect(finishedLabel(job({}))).toBeUndefined();
    expect(finishedLabel(job({ status: "done", finishedAt: "2026-11-19T10:05:00Z" }))).toMatch(/^Finished /);
  });

  it("says what an ended import did, in one line", () => {
    expect(importNotice(job({ status: "done", imported: 33019, alreadyInBridge: 12, failed: 2 })))
      .toEqual({ title: "Import finished", body: "33,019 items came over from Claude Code, 12 already in Bridge. 2 could not be imported.", failed: false });
    expect(importNotice(job({ status: "done", imported: 0, alreadyInBridge: 608 })).body).toBe("Nothing new came over from Claude Code. 608 already in Bridge.");
    expect(importNotice(job({ status: "done", imported: 1 })).body).toBe("1 item came over from Claude Code.");
    expect(importNotice(job({ status: "failed", errors: ["Claude Code was not found on this Mac"] })))
      .toEqual({ title: "Import stopped", body: "Claude Code was not found on this Mac", failed: true });
  });
});
