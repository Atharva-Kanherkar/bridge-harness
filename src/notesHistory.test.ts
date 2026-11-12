import { describe, expect, it } from "vitest";
import { historyEntries, lineDiff, versionLabel } from "./notesHistory";
import type { NoteVersion } from "./types";

const at = (minutes: number) => new Date(Date.UTC(2026, 10, 1, 9, minutes)).toISOString();
const version = (seq: number, actor: string, minutes: number, over: Partial<NoteVersion> = {}): NoteVersion =>
  ({ seq, actor, at: at(minutes), bodyChanged: true, added: 1, removed: 0, title: null, ...over });

describe("historyEntries", () => {
  it("folds a run by one author within ten minutes into its newest version", () => {
    // Newest first, as the core sends it.
    const versions = [
      version(7, "person", 40, { added: 2 }),
      version(6, "agent:claude:s1", 31, { added: 120 }),
      version(5, "person", 30, { bodyChanged: false }),
      version(4, "person", 9, { added: 3, removed: 1 }),
      version(3, "person", 5, { added: 4 }),
      version(2, "person", 0, { added: 10 }),
    ];
    const entries = historyEntries(versions, id => id === "s1" ? "Research sync" : undefined);
    expect(entries.map(entry => [entry.seq, entry.label, entry.added, entry.removed, entry.count])).toEqual([
      [7, "You", 2, 0, 1],
      [6, "Claude in Research sync", 120, 0, 1],
      [4, "You", 17, 1, 3],
    ]);
  });

  it("starts a new entry after ten quiet minutes", () => {
    const entries = historyEntries([version(3, "person", 25), version(2, "person", 9), version(1, "person", 0)], () => undefined);
    expect(entries.map(entry => [entry.seq, entry.count])).toEqual([[3, 1], [2, 2]]);
  });
});

describe("versionLabel", () => {
  it("names who wrote a version", () => {
    const chats = (id: string) => (id === "s1" ? "Research sync" : undefined);
    expect(versionLabel("person", chats)).toBe("You");
    expect(versionLabel("agent:codex:s1", chats)).toBe("Codex in Research sync");
    expect(versionLabel("agent:claude:gone", chats)).toBe("Claude in a chat");
    expect(versionLabel("file-import", chats)).toBe("Edited outside Bridge");
    expect(versionLabel("model:tasks:claude-sonnet-5-5", chats)).toBe("Turn into tasks");
    expect(versionLabel("model:structure:haiku", chats)).toBe("Structure");
  });
});

describe("lineDiff", () => {
  it("marks added and removed lines and keeps the rest", () => {
    expect(lineDiff("a\nb\nc\n", "a\nB\nc\nd\n")).toEqual([
      { kind: "same", text: "a" },
      { kind: "removed", text: "b" },
      { kind: "added", text: "B" },
      { kind: "same", text: "c" },
      { kind: "added", text: "d" },
    ]);
    expect(lineDiff("same", "same")).toEqual([{ kind: "same", text: "same" }]);
  });
});
