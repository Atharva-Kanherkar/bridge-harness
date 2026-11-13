// Completes and draws a scene exactly as a page's drawing preview does
// (`DrawingPreview` in src/components/notes/editor/drawingCanvas.tsx): the
// same skeleton completion, Excalidraw's own text measuring, the same SVG
// export at the note column's width. score.mjs calls `window.benchRender`.
import "../../src/components/notes/editor/drawingAssets";
import { Excalidraw, convertToExcalidrawElements, exportToSvg } from "@excalidraw/excalidraw";
import "@excalidraw/excalidraw/index.css";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { completeScene } from "../../src/components/notes/editor/drawingScene";

const convert = (skeletons, opts) => convertToExcalidrawElements(skeletons, opts);

// Mount Excalidraw once with text in every font a drawing uses, so its fonts
// are loaded before any label is measured, as they are in the editor.
const warm = [5, 1, 2, 3, 6, 7, 8].map((fontFamily, i) => ({ type: "text", id: `warm${i}`, x: 0, y: i * 30, text: "Warm up the font", fontSize: 20, fontFamily }));
window.benchReady = new Promise(resolve => {
  createRoot(document.getElementById("editor")).render(createElement(Excalidraw, {
    initialData: { elements: convert(warm, { regenerateIds: false }) },
    excalidrawAPI: () => setTimeout(async () => {
      await document.fonts.ready;
      await Promise.all(["20px Excalifont", "16px Excalifont", "28px Excalifont"].map(font => document.fonts.load(font).catch(() => [])));
      resolve(document.fonts.check("20px Excalifont"));
    }, 300),
  }));
});

window.benchRender = async scene => {
  await window.benchReady;
  const elements = completeScene(scene, convert);
  const svg = await exportToSvg({
    elements,
    appState: { ...(scene.appState ?? {}), exportBackground: true, viewBackgroundColor: "#ffffff", exportWithDarkMode: false },
    files: scene.files ?? {},
    exportPadding: 16,
  });
  svg.removeAttribute("width");
  svg.removeAttribute("height");
  // The preview's `block h-auto max-h-[28rem] w-full`.
  svg.setAttribute("style", "display:block;width:100%;height:auto;max-height:448px");
  document.getElementById("preview").replaceChildren(svg);
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const rect = svg.getBoundingClientRect();
  return { elements, rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } };
};
