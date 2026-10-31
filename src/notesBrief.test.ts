import { describe, expect, it } from "vitest";
import { actorLabel, notesContext, splitNotesContext, taskBrief, withNotesContext } from "./notesBrief";
import { parseAgentMention } from "./agentMention";
import { parseFindCommand } from "./chatSearch";
import { parseHarnessShortcut } from "./harnessShortcut";
import type { Note } from "./types";

const note = (over: Partial<Note> = {}): Note => ({
  id: "n1", spaceId: "personal", kind: "note", state: "inbox", tags: [], sessionIds: [], title: "Launch", createdAt: "", updatedAt: "",
  createdBy: "person", lastActor: "person", preview: "", seq: 4, ...over,
});

describe("taskBrief", () => {
  it("carries the task, its quote, the page it came from and the define-done ask", () => {
    const brief = taskBrief({ title: "Add a skip button", body: "Add a skip button to step 3.\n\nDone when: step 3 can be skipped.", excerpt: "- add a skip button\n- on step 3", parentTitle: "Onboarding" });
    expect(brief.startsWith("Task: Add a skip button\n\nAdd a skip button to step 3.")).toBe(true);
    expect(brief).toContain("Done when: step 3 can be skipped.");
    expect(brief).toContain('From my note "Onboarding":\n> - add a skip button\n> - on step 3');
    expect(brief).toContain("say in one line what done means");
    expect(brief).toContain("Ask me only about ambiguity that would change the outcome");
  });

  it("never starts with something the composer or core would run", () => {
    // The composer reads a leading / $ # or @ as a command, and so does the
    // core's slash dispatch, so a note's title must never open the message.
    for (const title of ["/find syntax", "$codex hi", "#reviewer go", "@session:abc", "/note buy milk", "  /compact now"]) {
      for (const kind of ["task", "note"] as const) {
        const brief = taskBrief({ kind, title, body: "do it" });
        expect(brief.trimStart()[0], `${kind}: ${title}`).toMatch(/[A-Za-z]/);
        expect(parseFindCommand(brief)).toBeNull();
        expect(parseHarnessShortcut(brief)).toBeNull();
        expect(parseAgentMention(brief)).toBeNull();
        expect(brief).toContain(title.trim());
      }
    }
  });

  it("leaves out what it does not have", () => {
    const brief = taskBrief({ title: "Pricing page", body: "Pricing page", excerpt: null, parentTitle: null });
    expect(brief.split("\n\n")).toHaveLength(2);
    expect(brief).not.toContain("From my note");
  });

  it("clips a very long body", () => {
    expect(taskBrief({ title: "t", body: "x".repeat(9000) })).toContain("[clipped: 3000 more characters]");
  });
});

describe("actorLabel", () => {
  const chats: Record<string, string> = { s1: "Research sync" };
  const title = (id: string) => chats[id];
  it("names the harness and the chat an agent wrote from", () => {
    expect(actorLabel("agent:claude:s1", title)).toBe("Written by Claude in Research sync");
    expect(actorLabel("agent:codex:gone", title)).toBe("Written by Codex in a chat");
  });
  it("labels file imports and stays quiet for the person and model passes", () => {
    expect(actorLabel("file-import", title)).toBe("Edited outside Bridge");
    expect(actorLabel("person", title)).toBeNull();
    expect(actorLabel("model:structure:claude-haiku-4-5", title)).toBeNull();
  });
});

describe("notes context", () => {
  it("round-trips through a message and leaves plain text alone", () => {
    const context = notesContext({ note: note({ tags: ["launch"] }), markdown: "# Launch\n```sh\nls\n```\n", projectName: "Bridge" });
    expect(context).toContain("Open page: Launch (id n1, seq 4)");
    expect(context).toContain("Project: Bridge · Tags: launch");
    const message = withNotesContext("  what is left?  ", context);
    expect(message.startsWith("what is left?\n\n````notes-context\n")).toBe(true);
    expect(splitNotesContext(message)).toEqual({ text: "what is left?", context });
    expect(splitNotesContext("no fence here")).toEqual({ text: "no fence here", context: null });
    expect(withNotesContext("just this", "")).toBe("just this");
  });
  it("clips a long page", () => {
    expect(notesContext({ note: note(), markdown: "y".repeat(5000) })).toContain("[clipped: 1000 more characters]");
  });
});
