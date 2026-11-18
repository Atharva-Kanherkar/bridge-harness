import { describe, expect, it } from "vitest";
import type { DetectedImportSource, ExternalImportArtifact, ExternalImportCandidate, ExternalImportCommit } from "./protocol/generated/protocol";
import { addBatch, chunk, emptyReport, folderLabel, groupChats, importableIds, sourceSummary } from "./moveToBridge";

const source = (counts: Partial<DetectedImportSource["counts"]>, installed = true): DetectedImportSource => ({
  tool: "claude_code", label: "Claude Code", installed, supported: true, roots: [], note: null,
  counts: { chats: 0, projects: 0, workspaces: 0, memories: 0, setup: 0, background: 0, ...counts },
});

const chat = (id: string, projectPath: string | null, startedAt: string | null, title: string | null = null): ExternalImportArtifact => ({
  artifactId: id, canonicalSourceRef: `/x/${id}.jsonl`, sourceLabel: `projects/p/${id}.jsonl`, kind: "conversation",
  classification: "version_gated_private", stability: "version_gated", estimatedBytes: 10, modifiedAt: null,
  requiredSchemaGate: "claude-jsonl-v2", title, projectPath, startedAt,
});

const candidate = (kind: ExternalImportCandidate["kind"], diagnostics: ExternalImportCandidate["diagnostics"] = []): ExternalImportCandidate => ({
  candidateId: `c-${Math.random()}`, source: { provider: "claude_code", adapterVersion: "2", sourceVersion: null, schemaVersion: null, canonicalSourceRef: "/x", sourcePathFingerprint: "f", discoveredAt: "now", sourceMetadata: {} },
  sourceNativeId: null, kind, title: "t", createdAt: null, updatedAt: null, projectHint: null, contentHash: "h", stability: "version_gated",
  confidenceBps: 8000, selectedByDefault: false, redactionSummary: { structuredFieldsExcluded: 0, textValuesRedacted: 0, categories: [], safelyRepresentable: true },
  diagnostics, normalizedPayload: {},
});

const commit = (fields: Partial<ExternalImportCommit>): ExternalImportCommit => ({
  importId: "i", candidateResults: [], createdBridgeIds: [], imported: 0, skipped: 0, changed: 0, conflicted: 0, rejected: 0, unsupported: 0,
  rollbackState: "committed", diagnostics: [], createdAt: "now", ...fields,
});

describe("Move to Bridge logic", () => {
  it("summarizes what a tool holds, and says when it is absent or empty", () => {
    expect(sourceSummary(source({ chats: 1312, projects: 14, memories: 3, setup: 1 }))).toBe("1,312 chats in 14 projects · 3 memories · 1 setup item");
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

  it("groups chats by folder, newest project and newest chat first, with fallbacks for missing titles", () => {
    const groups = groupChats([
      chat("a", "/w/old", "2026-01-01T00:00:00Z", "Old one"),
      chat("b", "/w/new", "2026-11-01T00:00:00Z", "  "),
      chat("c", "/w/new", "2026-11-02T00:00:00Z", "Newest"),
      chat("d", null, null),
      { ...chat("m", "/w/new", null), kind: "memory" },
    ]);
    expect(groups.map(group => group.label)).toEqual(["/w/new", "/w/old", "Folder unknown"]);
    expect(groups[0].chats.map(row => row.title)).toEqual(["Newest", "b.jsonl"]);
    expect(groups[0].chats).toHaveLength(2);
  });

  it("puts temp-folder sessions in one Background runs group, last", () => {
    const groups = groupChats([
      chat("p", "/private/var/folders/h7/x/T", "2026-12-01T00:00:00Z"),
      chat("q", "/tmp/scratch", "2026-12-02T00:00:00Z"),
      chat("r", "/work/app", "2026-01-01T00:00:00Z"),
    ]);
    expect(groups.map(group => [group.label, group.chats.length, group.background])).toEqual([["/work/app", 1, false], ["Background runs", 2, true]]);
  });

  it("chunks into batches of 25 and keeps only importable candidates", () => {
    expect(chunk(Array.from({ length: 51 }, (_, i) => i)).map(batch => batch.length)).toEqual([25, 25, 1]);
    expect(chunk([])).toEqual([]);
    const kept = candidate("conversation");
    expect(importableIds([kept, candidate("unsupported")])).toEqual([kept.candidateId]);
  });

  it("reports skipped transcript records in what did not come over, but not removed repeats", () => {
    const skipped = (reason: string, count: number) => ({ code: "transcript_record_skipped", severity: "info", classification: "documented" as const, message: `Skipped ${count} '${reason}' record(s) Bridge has no place for`, recovery: null, sourceLabel: "projects/p/s.jsonl" });
    const report = addBatch(emptyReport(), [
      candidate("conversation", [skipped("content:document", 1), skipped("tool_result_without_call", 2), skipped("malformed_line", 3), skipped("record:brand-new", 1), skipped("duplicate_source_id", 5)]),
      candidate("conversation", [skipped("content:document", 2)]),
    ], commit({ imported: 2 }));
    expect(report.imported).toBe(2);
    expect(report.notCarried).toEqual([
      { text: "documents or PDFs inside messages", count: 3 },
      { text: "tool results with no matching call", count: 2 },
      { text: "unreadable lines", count: 3 },
      { text: "records of a kind Bridge does not know (brand-new)", count: 1 },
    ]);
  });

  it("adds up batches: counts, unreadable chats, and what did not come over by reason", () => {
    let report = emptyReport();
    report = addBatch(report, [
      candidate("conversation", [{ code: "excluded_attachment", severity: "warning", classification: "documented", message: "2 attachment: image too large", recovery: null, sourceLabel: "a" }]),
      candidate("unsupported", [{ code: "artifact_preview_failed", severity: "warning", classification: "unsupported_private", message: "transcript is unreadable", recovery: null, sourceLabel: "b" }]),
    ], commit({ imported: 3, skipped: 1 }));
    report = addBatch(report, [
      candidate("conversation", [
        { code: "excluded_attachment", severity: "warning", classification: "documented", message: "1 attachment: image too large", recovery: null, sourceLabel: "c" },
        { code: "transcript_record_skipped", severity: "info", classification: "documented", message: "Skipped 3 'content:document' record(s) Bridge has no place for", recovery: null, sourceLabel: "c" },
      ]),
    ], commit({ imported: 2, changed: 1, rejected: 1, diagnostics: [{ code: "batch_failed", severity: "error", classification: "documented", message: "Batch 2 rolled back", recovery: null, sourceLabel: null }] }));
    report = addBatch(report, [], null, "The Bridge daemon connection was lost");
    expect(report.imported).toBe(5);
    expect(report.alreadyInBridge).toBe(1);
    expect(report.changed).toBe(1);
    expect(report.failed).toBe(2);
    expect(report.notCarried).toEqual([
      { text: "attachment: image too large", count: 3 },
      { text: "A chat could not be read: transcript is unreadable", count: 1 },
      { text: "documents or PDFs inside messages", count: 3 },
    ]);
    expect(report.errors).toEqual(["Batch 2 rolled back", "The Bridge daemon connection was lost"]);
  });
});
