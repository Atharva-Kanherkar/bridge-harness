import { describe, expect, it } from "vitest";
import { partialJson, streamingStructure, streamingTasks } from "./notesPass";

const reply = JSON.stringify({
  title: "Beta, then the launch post",
  kind: "task",
  tags: ["launch", "beta"],
  summary: "Ship the beta to ten people, then write the post.",
  items: [
    { excerpt: "ship the beta", title: "Ship the beta", kind: "task" },
    { excerpt: "write the post", title: "Write the launch post", kind: "task" },
  ],
  project: null,
});

describe("partialJson", () => {
  it("reads every prefix of a reply without throwing, and the whole reply exactly", () => {
    for (let end = 0; end <= reply.length; end += 1) partialJson(reply.slice(0, end));
    expect(partialJson(reply)).toEqual(JSON.parse(reply));
  });

  it("closes the string being written", () => {
    expect(partialJson('{"title": "Beta, th')).toEqual({ title: "Beta, th" });
    expect(partialJson('{"title": "a\\"b')).toEqual({ title: "a\"b" });
    expect(partialJson('{"title": "line\\')).toEqual({ title: "line" });
  });

  it("drops a key with no value yet, and a number or literal still being written", () => {
    expect(partialJson('{"title": "x", "summ')).toEqual({ title: "x" });
    expect(partialJson('{"title": "x", "summary":')).toEqual({ title: "x" });
    expect(partialJson('{"n": 1, "ok": tr')).toEqual({ n: 1 });
  });

  it("skips a code fence or prose before the object", () => {
    expect(partialJson('```json\n{"title": "x"')).toEqual({ title: "x" });
  });

  it("returns null for text with no object", () => {
    expect(partialJson("")).toBeNull();
    expect(partialJson("I can't help with that.")).toBeNull();
  });
});

describe("streamingStructure", () => {
  it("fills in fields as they arrive", () => {
    const cut = reply.indexOf("Ship the beta to");
    const early = streamingStructure(reply.slice(0, cut + 4));
    expect(early).toMatchObject({ title: "Beta, then the launch post", kind: "task", tags: ["launch", "beta"], summary: "Ship", items: [] });
    const later = streamingStructure(reply.slice(0, reply.indexOf("Write the launch") + 5));
    expect(later?.items.map(item => item.title)).toEqual(["Ship the beta", "Write"]);
    expect(streamingStructure(reply)?.items).toHaveLength(2);
  });
});

describe("streamingTasks", () => {
  it("lists task titles as they arrive", () => {
    const tasks = JSON.stringify({ tasks: [{ excerpt: "a", title: "Book the venue", detail: "", doneWhen: "", project: null }, { excerpt: "b", title: "Send invites" }] });
    expect(streamingTasks(tasks.slice(0, tasks.indexOf("Send") + 2))).toEqual([{ title: "Book the venue", excerpt: "a" }, { title: "Se", excerpt: "b" }]);
    expect(streamingTasks('{"tasks": [')).toEqual([]);
    expect(streamingTasks("nope")).toEqual([]);
  });
});
