// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import { completeScene, emptyScene, markDrawings, mergeScenes, parseScene, repairBindings, serializeScene, type ConvertSkeletons, type SceneFile, type StoredElement } from "./drawingScene";

const fixture = (name: string): SceneFile => parseScene(readFileSync(resolve(__dirname, "../../../../testing/fixtures/excalidraw", name), "utf8"))!;
const byId = (elements: StoredElement[], id: string) => elements.find(element => element.id === id)!;
const boundTo = (element: StoredElement) => ((element.boundElements as { id: string }[] | null) ?? []).map(entry => entry.id);

let convert: ConvertSkeletons;

beforeAll(async () => {
  // jsdom has no canvas or FontFace; Excalidraw only measures text with them.
  HTMLCanvasElement.prototype.getContext = function () {
    return { filter: "none", font: "", measureText: (text: string) => ({ width: text.length * 10, actualBoundingBoxAscent: 8, actualBoundingBoxDescent: 2 }) };
  } as never;
  (globalThis as { FontFace?: unknown }).FontFace = class { load() { return Promise.resolve(this); } };
  const excalidraw = await import("@excalidraw/excalidraw");
  convert = (skeletons, opts) => excalidraw.convertToExcalidrawElements(skeletons as never, opts) as never;
}, 120_000);

describe("parseScene", () => {
  it("reads a scene, a bare element list and an empty body, and refuses anything else", () => {
    expect(parseScene("")).toEqual(emptyScene());
    expect(parseScene('[{"id":"a","type":"rectangle"}]')!.elements).toHaveLength(1);
    expect(parseScene('{"elements":[{"id":"a","type":"rectangle"},{"nope":1}]}')!.elements).toHaveLength(1);
    expect(parseScene("graph TD; A-->B")).toBeNull();
    expect(parseScene('{"type":"excalidraw"}')).toBeNull();
  });
});

describe("completeScene", () => {
  it("completes what notes_draw wrote into bound, labelled Excalidraw elements", () => {
    const scene = fixture("agent-login.excalidraw");
    const elements = completeScene(scene, convert);
    // Every element whole, every agent id kept.
    expect(elements.every(element => typeof element.version === "number")).toBe(true);
    for (const id of ["form", "check", "home", "retry", "hint", "e1", "e2", "e3"]) expect(byId(elements, id)).toBeTruthy();
    // Labels are named after their container, so a second pass names them the same.
    expect(byId(elements, "form~label")).toMatchObject({ type: "text", containerId: "form", text: "Login form" });
    expect(byId(elements, "e2~label")).toMatchObject({ containerId: "e2", text: "yes" });
    expect(completeScene(scene, convert).map(element => element.id)).toEqual(elements.map(element => element.id));
    // Arrows bind both ends, and each end lists the arrow.
    expect(byId(elements, "e1")).toMatchObject({ startBinding: { elementId: "form" }, endBinding: { elementId: "check" } });
    expect(boundTo(byId(elements, "check"))).toEqual(expect.arrayContaining(["check~label", "e1", "e2", "e3"]));
    expect(byId(elements, "e3").strokeStyle).toBe("dashed");
    expect(byId(elements, "home").backgroundColor).toBe("#b2f2bb");
  });

  it("leaves a person's drawing exactly as drawn, without what they deleted", () => {
    const scene = fixture("human-architecture.excalidraw");
    const elements = completeScene(scene, convert);
    expect(elements).toEqual(scene.elements.filter(element => !element.isDeleted));
  });

  it("binds an agent's arrow to a box the person drew, and keeps that box's label", () => {
    const elements = completeScene(fixture("human-plus-agent.excalidraw"), convert);
    expect(byId(elements, "c1")).toMatchObject({ startBinding: { elementId: "api" }, endBinding: { elementId: "cache" } });
    expect(boundTo(byId(elements, "api"))).toEqual(expect.arrayContaining(["api-label", "a1", "c1"]));
    expect(elements.filter(element => element.containerId === "api")).toHaveLength(1);
    // The relabelled box got its new label once, and the old one is gone.
    expect(elements.filter(element => element.containerId === "db").map(element => element.text)).toEqual(["Postgres 16"]);
    expect(elements.some(element => element.id === "legacy" || element.id === "a3")).toBe(false);
    // The person's freehand loop is untouched.
    expect(byId(elements, "circle")).toEqual(byId(fixture("human-architecture.excalidraw").elements, "circle"));
  });
});

describe("mergeScenes", () => {
  const scenes = () => {
    const baseScene = fixture("human-architecture.excalidraw");
    const base = { stored: baseScene.elements, complete: completeScene(baseScene, convert) };
    const remoteScene = fixture("human-plus-agent.excalidraw");
    const remote = { stored: remoteScene.elements, complete: completeScene(remoteScene, convert) };
    return { base, remote };
  };
  const bump = (element: StoredElement, change: Partial<StoredElement>) => ({ ...element, ...change, version: (element.version ?? 0) + 1 });

  it("keeps the person's edits and the agent's, each where only one side changed", () => {
    const { base, remote } = scenes();
    // While the agent worked, the person moved the web box and deleted the note.
    const local = base.complete.filter(element => element.id !== "note").map(element => element.id === "web" ? bump(element, { x: 0 }) : element);
    const merged = mergeScenes(base, local, remote);
    expect(byId(merged, "web").x).toBe(0);
    expect(merged.some(element => element.id === "note")).toBe(false);
    // The agent's additions, relabel and removal all land.
    expect(byId(merged, "cache")).toBeTruthy();
    expect(byId(merged, "c1")).toBeTruthy();
    expect(merged.filter(element => element.containerId === "db").map(element => element.text)).toEqual(["Postgres 16"]);
    expect(merged.some(element => element.id === "legacy" || element.id === "legacy-label" || element.id === "a3")).toBe(false);
    expect(merged.every(element => !boundTo(element).includes("a3"))).toBe(true);
  });

  it("lets the person win where both changed the same element", () => {
    const { base, remote } = scenes();
    const local = base.complete.map(element => element.id === "db" ? bump(element, { backgroundColor: "#ffc9c9" }) : element);
    const merged = mergeScenes(base, local, remote);
    expect(byId(merged, "db").backgroundColor).toBe("#ffc9c9");
    // Something the person kept editing survives the agent removing it.
    const kept = mergeScenes(base, base.complete.map(element => element.id === "legacy" ? bump(element, { x: 999 }) : element), remote);
    expect(byId(kept, "legacy").x).toBe(999);
  });

  it("changes nothing when neither side did", () => {
    const { base } = scenes();
    expect(mergeScenes(base, base.complete, base)).toEqual(base.complete);
  });
});

describe("serializeScene", () => {
  it("writes the core's layout, one element per line, and round-trips", () => {
    const raw = readFileSync(resolve(__dirname, "../../../../testing/fixtures/excalidraw/agent-login.excalidraw"), "utf8").trimEnd();
    const scene = parseScene(raw)!;
    const body = serializeScene(scene.elements, scene.appState, scene.files);
    expect(body).toBe(raw);
    expect(serializeScene([])).toBe('{"type":"excalidraw","version":2,"source":"bridge","elements":[],"appState":{},"files":{}}');
  });

  it("drops dangling bindings on the way", () => {
    const repaired = repairBindings([{ id: "a", type: "arrow", version: 1, startBinding: { elementId: "gone" }, boundElements: [{ id: "x", type: "text" }] }]);
    expect(repaired[0]).toMatchObject({ startBinding: null, boundElements: null });
  });
});

describe("markDrawings", () => {
  it("swaps each drawing's JSON for a marker and leaves other fences alone", () => {
    const page = "Intro\n\n```excalidraw\n{\"elements\":[]}\n```\n\n```js\nx\n```\n\n````excalidraw\n{}\n````\nEnd";
    const marked = markDrawings(page);
    expect(marked).toContain("```excalidraw\n[drawing 1: call notes_read");
    expect(marked).toContain("````excalidraw\n[drawing 2:");
    expect(marked).toContain("```js\nx\n```");
    expect(marked).not.toContain("elements");
    expect(marked.endsWith("End")).toBe(true);
  });
});
