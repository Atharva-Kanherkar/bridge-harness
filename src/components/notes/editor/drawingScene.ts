// A drawing's scene, kept pure for tests: what a ```excalidraw fence holds,
// how its agent-written skeletons become whole Excalidraw elements, and how a
// person's edits and an agent's edits to the same drawing meet.
//
// The fence body is an `.excalidraw` file's JSON. Elements a person drew are
// whole (they carry `version`); elements an agent added with `notes_draw` are
// Excalidraw skeletons (a `label`, an arrow's `start`/`end` ids, no
// `version`). The core writes the same one-element-per-line layout as
// `serializeScene`, so either side's change is a line or two in the history.

export const DRAWING_FENCE = "excalidraw";

/** An element as stored: whole or a skeleton. */
export interface StoredElement {
  id: string;
  type: string;
  version?: number;
  isDeleted?: boolean;
  [key: string]: unknown;
}

export interface SceneFile {
  type?: string;
  version?: number;
  source?: string;
  elements: StoredElement[];
  appState?: Record<string, unknown>;
  files?: Record<string, unknown>;
}

/** Excalidraw's `convertToExcalidrawElements`, passed in so tests need no canvas. */
export type ConvertSkeletons = (skeletons: StoredElement[], opts: { regenerateIds: boolean }) => readonly StoredElement[];

export const emptyScene = (): SceneFile => ({ type: "excalidraw", version: 2, source: "bridge", elements: [], appState: { viewBackgroundColor: "#ffffff" }, files: {} });

/** The body a new, empty drawing starts with. */
export const newDrawingBody = () => serializeScene([], emptyScene().appState);

/** A fence body as a scene. An empty body is an empty scene; a body that is
 *  not a scene is null, and the block shows it as code. */
export function parseScene(body: string): SceneFile | null {
  if (!body.trim()) return emptyScene();
  try {
    const value: unknown = JSON.parse(body);
    if (Array.isArray(value)) return { ...emptyScene(), elements: value.filter(isElement) };
    if (value && typeof value === "object" && Array.isArray((value as SceneFile).elements)) {
      const scene = value as SceneFile;
      return { ...scene, elements: scene.elements.filter(isElement) };
    }
  } catch { /* not JSON */ }
  return null;
}

const isElement = (value: unknown): value is StoredElement =>
  !!value && typeof value === "object" && typeof (value as StoredElement).id === "string" && typeof (value as StoredElement).type === "string";

export const isWhole = (element: StoredElement) => typeof element.version === "number";

/** The ids a skeleton leans on: its arrow ends. */
function skeletonRefs(element: StoredElement): string[] {
  const ends = [element.start, element.end] as ({ id?: unknown } | undefined)[];
  return ends.map(end => end?.id).filter((id): id is string => typeof id === "string");
}

/**
 * Whole elements, ready for Excalidraw. Skeletons are completed with their
 * ids kept; a whole element an arrow skeleton binds to is completed beside it,
 * so the arrow and the element both record the binding. Order is kept.
 */
export function completeScene(scene: SceneFile, convert: ConvertSkeletons): StoredElement[] {
  const elements = scene.elements.filter(element => !element.isDeleted);
  const skeletons = elements.filter(element => !isWhole(element));
  if (!skeletons.length) return elements;
  const byId = new Map(elements.map(element => [element.id, element]));
  const leaned = new Set(skeletons.flatMap(skeletonRefs).filter(id => byId.has(id) && isWhole(byId.get(id)!)));
  // A whole element going through the converter as a skeleton keeps its
  // fields; only its own label must not come back twice.
  const batch = elements.filter(element => !isWhole(element) || leaned.has(element.id));
  const converted = convert(batch, { regenerateIds: false });
  // The converter names each new label at random; name it after its
  // container instead, so completing the same scene twice gives the same ids
  // and a merge can tell an unchanged label from a new one.
  const renamed = new Map<string, string>();
  for (const element of converted) {
    if (!byId.has(element.id) && element.type === "text" && typeof element.containerId === "string") renamed.set(element.id, `${element.containerId}~label`);
  }
  const made = new Map<string, StoredElement>();
  const labels = new Map<string, StoredElement>();
  const extra: StoredElement[] = [];
  for (const raw of converted) {
    const element = renamed.size ? renameRefs(raw, renamed) : raw;
    if (byId.has(element.id)) made.set(element.id, element);
    else if (element.type === "text" && typeof element.containerId === "string") labels.set(element.containerId, element);
    else extra.push(element);
  }
  const out: StoredElement[] = [];
  for (const element of elements) {
    out.push(made.get(element.id) ?? element);
    const label = labels.get(element.id);
    if (label) out.push(label);
  }
  return repairBindings([...out, ...extra]);
}

function renameRefs(element: StoredElement, renamed: Map<string, string>): StoredElement {
  const id = renamed.get(element.id) ?? element.id;
  const bound = element.boundElements as { id: string; type: string }[] | null | undefined;
  return { ...element, id, ...(bound ? { boundElements: bound.map(entry => ({ ...entry, id: renamed.get(entry.id) ?? entry.id })) } : {}) };
}

/** Every arrow binding recorded on both ends, and nothing bound to a gap. */
export function repairBindings(elements: StoredElement[]): StoredElement[] {
  const ids = new Set(elements.map(element => element.id));
  const wanted = new Map<string, { id: string; type: string }[]>();
  const want = (target: string, entry: { id: string; type: string }) => {
    if (!ids.has(target)) return;
    const list = wanted.get(target) ?? [];
    if (!list.some(item => item.id === entry.id)) list.push(entry);
    wanted.set(target, list);
  };
  for (const element of elements) {
    for (const key of ["startBinding", "endBinding"] as const) {
      const target = (element[key] as { elementId?: string } | null | undefined)?.elementId;
      if (target) want(target, { id: element.id, type: "arrow" });
    }
    if (element.type === "text" && typeof element.containerId === "string") want(element.containerId, { id: element.id, type: "text" });
  }
  return elements.map(element => {
    const own = ((element.boundElements as { id: string; type: string }[] | null | undefined) ?? []).filter(entry => ids.has(entry.id));
    const add = (wanted.get(element.id) ?? []).filter(entry => !own.some(item => item.id === entry.id));
    const bindings = Object.fromEntries((["startBinding", "endBinding"] as const)
      .filter(key => { const target = (element[key] as { elementId?: string } | null | undefined)?.elementId; return target && !ids.has(target); })
      .map(key => [key, null]));
    const bound = [...own, ...add];
    const changed = add.length || own.length !== ((element.boundElements as unknown[] | null | undefined)?.length ?? 0) || Object.keys(bindings).length;
    return changed ? { ...element, ...bindings, boundElements: bound.length ? bound : null } : element;
  });
}

const same = (a: StoredElement | undefined, b: StoredElement | undefined) => JSON.stringify(a) === JSON.stringify(b);

/**
 * Three-way merge by element id, for a drawing changed in two places since
 * `base`: the person's open editor (`local`, whole elements) and the page
 * (`remote`, as stored, perhaps an agent's skeletons). A side that changed an
 * element wins over a side that did not; when both did, the person's stands.
 * An element one side added is kept; one a side removed, and the other left
 * alone, goes. Local changes are judged by Excalidraw's `version`, remote
 * ones by the stored JSON.
 */
export function mergeScenes(base: { stored: StoredElement[]; complete: StoredElement[] }, local: StoredElement[], remote: { stored: StoredElement[]; complete: StoredElement[] }): StoredElement[] {
  const baseStored = new Map(base.stored.map(element => [element.id, element]));
  const baseVersion = new Map(base.complete.map(element => [element.id, element.version]));
  const remoteStored = new Map(remote.stored.map(element => [element.id, element]));
  const remoteComplete = new Map(remote.complete.map(element => [element.id, element]));
  const localById = new Map(local.filter(element => !element.isDeleted).map(element => [element.id, element]));
  // A bound label rides with its container, judged by the container.
  const owner = (element: StoredElement | undefined) => typeof element?.containerId === "string" ? element.containerId : element?.id;
  const remoteChanged = (id: string) => {
    const element = remoteComplete.get(id) ?? base.complete.find(item => item.id === id);
    const key = owner(element) ?? id;
    return !same(baseStored.get(key), remoteStored.get(key));
  };
  const localChanged = (id: string) => {
    const element = localById.get(id);
    return !element || element.version !== baseVersion.get(id);
  };
  const out: StoredElement[] = [];
  const seen = new Set<string>();
  const take = (element: StoredElement) => { if (!seen.has(element.id)) { seen.add(element.id); out.push(element); } };
  for (const element of local) {
    if (element.isDeleted) continue;
    const id = element.id;
    const inBase = baseVersion.has(id);
    const inRemote = remoteComplete.has(id);
    if (!inBase) take(element);
    else if (inRemote) take(remoteChanged(id) && !localChanged(id) ? remoteComplete.get(id)! : element);
    else if (localChanged(id)) take(element); // the page dropped it, the person changed it: keep
  }
  // New on the page. One the person deleted stays deleted.
  for (const element of remote.complete) if (!baseVersion.has(element.id)) take(element);
  return repairBindings(out);
}

/** A scene as a fence body: one element per line, the core's layout. */
export function serializeScene(elements: readonly StoredElement[], appState: Record<string, unknown> = {}, files: Record<string, unknown> = {}): string {
  const head = `{"type":"excalidraw","version":2,"source":"bridge","elements":[`;
  const live = elements.filter(element => !element.isDeleted);
  const body = live.length ? `\n${live.map(element => JSON.stringify(element)).join(",\n")}\n]` : "]";
  return `${head}${body},"appState":${JSON.stringify(appState)},"files":${JSON.stringify(files)}}`;
}

/** The app state worth keeping with a drawing. */
export function keptAppState(appState: Record<string, unknown>): Record<string, unknown> {
  const kept: Record<string, unknown> = {};
  for (const key of ["viewBackgroundColor", "gridSize", "gridStep", "gridModeEnabled"]) if (appState[key] !== undefined) kept[key] = appState[key];
  return kept;
}

/** Files only the drawing's live image elements still use. */
export function keptFiles(elements: readonly StoredElement[], files: Record<string, unknown>): Record<string, unknown> {
  const used = new Set(elements.filter(element => !element.isDeleted && element.type === "image").map(element => element.fileId));
  return Object.fromEntries(Object.entries(files).filter(([id]) => used.has(id)));
}

/** A page's Markdown with each drawing's JSON swapped for a marker, for the
 *  docked Notes chat; `notes_read` describes the drawing. Mirrors the core's
 *  `drawing::for_agent` marker. */
export function markDrawings(markdown: string): string {
  let index = 0;
  return markdown.replace(/^( {0,3})(`{3,}|~{3,})[ \t]*excalidraw\b[^\n]*\n([\s\S]*?)(?:^ {0,3}\2[`~]*[ \t]*$|(?![\s\S]))/gim, (whole, indent: string, fence: string) => {
    const number = ++index;
    const closed = /\n {0,3}[`~]{3,}[ \t]*$/.test(whole);
    return `${indent}${fence}excalidraw\n[drawing ${number}: call notes_read on this page to see it described; change it with notes_draw, never notes_edit]\n${closed ? `${indent}${fence}` : ""}`;
  });
}
