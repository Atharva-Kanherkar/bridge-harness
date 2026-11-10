// The Excalidraw half of a drawing block, loaded only when a page holds a
// drawing: the package is large, and most pages never need it.
//
// `DrawingPreview` draws the scene as an SVG inside the page. `DrawingEditor`
// is Excalidraw itself, full screen. It saves as the person draws, and when the
// page's copy changes underneath (an agent's `notes_draw`, another window) it
// merges that change into the open canvas rather than painting over it.

import "./drawingAssets";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CaptureUpdateAction, Excalidraw, convertToExcalidrawElements, exportToSvg, getSceneVersion } from "@excalidraw/excalidraw";
import type { ExcalidrawImperativeAPI } from "@excalidraw/excalidraw/types";
import "@excalidraw/excalidraw/index.css";
import { Check } from "lucide-react";
import { completeScene, keptAppState, keptFiles, mergeScenes, parseScene, serializeScene, type ConvertSkeletons, type SceneFile, type StoredElement } from "./drawingScene";

const convert: ConvertSkeletons = (skeletons, opts) => convertToExcalidrawElements(skeletons as never, opts) as unknown as StoredElement[];
const SAVE_MS = 800;

export function DrawingPreview({ scene, dark }: { scene: SceneFile; dark: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true;
    setFailed(false);
    const elements = completeScene(scene, convert);
    exportToSvg({
      elements: elements as never,
      appState: { ...scene.appState, exportBackground: false, exportWithDarkMode: dark } as never,
      files: (scene.files ?? {}) as never,
      exportPadding: 16,
    }).then((svg: SVGSVGElement) => {
      if (!current || !host.current) return;
      // The node itself, never its markup re-parsed: labels are the person's text.
      svg.removeAttribute("width");
      svg.removeAttribute("height");
      svg.setAttribute("class", "block h-auto max-h-[28rem] w-full");
      svg.setAttribute("role", "img");
      host.current.replaceChildren(svg);
    }, () => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [scene, dark]);
  if (failed) return <p className="m-0 p-4 text-ui text-muted-foreground">This drawing could not be drawn.</p>;
  return <div ref={host} className="min-h-24" />;
}

export function DrawingEditor({ body, dark, title, onSave, onClose }: {
  /** The page's copy of the drawing, as it stands now. */
  body: string;
  dark: boolean;
  title?: string;
  onSave: (body: string) => void;
  onClose: () => void;
}) {
  const api = useRef<ExcalidrawImperativeAPI | null>(null);
  // `loaded` once Excalidraw shows the opened scene, `dirty` once the person
  // changed it: nothing is saved before both, so a canvas still loading (or
  // React's StrictMode remount) never writes an empty drawing over the page.
  const opened = useRef<{ base: { stored: StoredElement[]; complete: StoredElement[] }; written: string; version: number; loaded: boolean; dirty: boolean } | null>(null);
  if (!opened.current) {
    const scene = parseScene(body) ?? parseScene("")!;
    const complete = completeScene(scene, convert);
    opened.current = { base: { stored: scene.elements, complete }, written: body, version: getSceneVersion(complete as never), loaded: false, dirty: false };
  }
  const [initial] = useState(() => {
    const scene = parseScene(body) ?? parseScene("")!;
    return { elements: opened.current!.base.complete as never, appState: { ...scene.appState, theme: dark ? "dark" : "light" } as never, files: (scene.files ?? {}) as never, scrollToContent: true };
  });
  const timer = useRef<number | null>(null);

  const flush = () => {
    if (timer.current !== null) { window.clearTimeout(timer.current); timer.current = null; }
    const canvas = api.current;
    const state = opened.current;
    if (!canvas || !state || !state.dirty) return;
    state.dirty = false;
    const elements = canvas.getSceneElements() as unknown as StoredElement[];
    const text = serializeScene(elements, keptAppState(canvas.getAppState() as never), keptFiles(elements, canvas.getFiles() as never));
    if (text === state.written) return;
    state.written = text;
    state.base = { stored: [...elements], complete: [...elements] };
    onSave(text);
  };

  // The page changed under the open canvas: fold that change in.
  useEffect(() => {
    const state = opened.current;
    const canvas = api.current;
    if (!state || !canvas || body === state.written) return;
    const remote = parseScene(body);
    if (!remote) return;
    const remoteComplete = completeScene(remote, convert);
    const merged = mergeScenes(state.base, canvas.getSceneElementsIncludingDeleted() as unknown as StoredElement[], { stored: remote.elements, complete: remoteComplete });
    state.base = { stored: remote.elements, complete: remoteComplete };
    state.written = body;
    canvas.updateScene({ elements: merged as never, captureUpdate: CaptureUpdateAction.NEVER });
    state.version = getSceneVersion(merged as never);
  }, [body]);

  useEffect(() => api.current?.updateScene({ appState: { theme: dark ? "dark" : "light" } as never, captureUpdate: CaptureUpdateAction.NEVER }), [dark]);
  // Whatever is unsaved goes when the editor does.
  useEffect(() => () => flush(), []); // eslint-disable-line react-hooks/exhaustive-deps

  const changed = (elements: readonly unknown[]) => {
    const state = opened.current;
    const version = getSceneVersion(elements as never);
    if (!state) return;
    if (!state.loaded) {
      // Excalidraw restores the scene as it loads it, so its versions differ
      // from ours; the opened elements all being there is the sign it is in.
      const live = (elements as { isDeleted?: boolean }[]).filter(element => !element.isDeleted).length;
      if (live !== state.base.complete.length) return;
      state.loaded = true;
      state.version = version;
      return;
    }
    if (version === state.version) return;
    state.version = version;
    state.dirty = true;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(flush, SAVE_MS);
  };

  return createPortal(<div role="dialog" aria-modal="true" aria-label={title ? `Drawing in ${title}` : "Drawing"} className="fixed inset-0 z-50 flex flex-col bg-background">
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
      <h2 className="m-0 min-w-0 flex-1 truncate text-[13px] font-semibold text-foreground">{title ? `Drawing in ${title}` : "Drawing"}</h2>
      <p className="m-0 hidden text-caption text-muted-foreground sm:block">Saved to the page as you draw. Agents can read and add to it.</p>
      <button type="button" onClick={() => { flush(); onClose(); }} className="inline-flex h-8 items-center gap-1.5 rounded-full bg-foreground px-3 text-[12px] font-medium text-background transition-opacity hover:opacity-90">
        <Check size={14} aria-hidden="true" />Done
      </button>
    </header>
    <div className="min-h-0 flex-1">
      <Excalidraw
        excalidrawAPI={instance => { api.current = instance; }}
        initialData={initial}
        theme={dark ? "dark" : "light"}
        onChange={changed}
        UIOptions={{ tools: { image: false }, canvasActions: { loadScene: false, saveToActiveFile: false, toggleTheme: null, export: false } }}
      />
    </div>
  </div>, document.body);
}
