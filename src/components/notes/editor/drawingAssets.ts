// Excalidraw's hand-drawn fonts come from Bridge itself (vite.config.ts serves
// and bundles them under /excalidraw/fonts), not from a CDN, so drawings look
// right offline. Imported before the package, which reads this as it loads.
// Only CJK text, whose font is too large to ship, still falls back to the CDN.

declare global {
  interface Window { EXCALIDRAW_ASSET_PATH?: string | string[] }
}

window.EXCALIDRAW_ASSET_PATH = "/excalidraw/";

export {};
