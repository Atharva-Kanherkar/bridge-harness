import { describe, expect, it } from "vitest";
import { latestTaskChat, noteTaskStatus } from "./notesTask";
import type { Note, Session, SessionStatus } from "./types";

const task = (props: Partial<Note> = {}) => ({ state: "open", sessionIds: [], ...props }) as Note;
const chat = (status: SessionStatus) => ({ id: "chat-new", status }) as Session;

describe("note task status", () => {
  it.each<[SessionStatus, string]>([
    ["idle", "Chat started"], ["starting", "Starting…"], ["working", "Running"],
    ["waiting", "Needs you"], ["warm", "Running"], ["checkpointing", "Saving progress…"],
    ["ready", "Ready to review"], ["stopped", "Stopped"], ["resuming", "Resuming…"],
    ["restored", "Resuming…"], ["failed", "Failed"], ["completed", "Finished"], ["cancelled", "Cancelled"],
  ])("describes a linked chat that is %s", (status, label) => {
    expect(noteTaskStatus(task({ state: "doing", sessionIds: ["chat-new"] }), chat(status))).toBe(label);
  });

  it("keeps manual completion and archival authoritative", () => {
    expect(noteTaskStatus(task({ state: "done" }), chat("working"))).toBe("Done");
    expect(noteTaskStatus(task({ state: "archived" }), chat("working"))).toBe("Archived");
    expect(noteTaskStatus(task({ state: "doing" }))).toBe("In progress");
    expect(noteTaskStatus(task())).toBe("Not started");
    expect(noteTaskStatus(task({ sessionIds: ["unloaded"] }))).toBe("Chat linked");
  });

  it("follows the newest chat link, including while its details load", () => {
    const old = { ...chat("completed"), id: "chat-old" };
    const chats = new Map([[old.id, old], ["chat-new", chat("working")]]);
    const note = task({ sessionIds: [old.id, "chat-new"] });
    expect(latestTaskChat(note, chats).session?.status).toBe("working");
    chats.delete("chat-new");
    expect(latestTaskChat(note, chats)).toEqual({ id: "chat-new", session: undefined });
    expect(latestTaskChat(task(), chats)).toEqual({ id: undefined, session: undefined });
  });
});
