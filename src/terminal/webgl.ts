// Adapted from Orca's pane-webgl-renderer.ts at f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
import type { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";

// Inactive tabs unmount presentation, retaining zero hidden GPU contexts.
const MAX_CONTEXTS = 8;
let contexts = 0;

export function attachWebgl(terminal: Terminal, refit: () => void): () => void {
  if (contexts >= MAX_CONTEXTS) return () => {};
  let addon: WebglAddon | null = null;
  let counted = false;
  let loss: { dispose: () => void } | undefined;
  const dispose = () => {
    loss?.dispose();
    if (addon) {
      try {
        const renderer = (addon as unknown as { _renderer?: { _gl?: WebGLRenderingContext; _canvas?: HTMLCanvasElement } })._renderer;
        renderer?._gl?.getExtension("WEBGL_lose_context")?.loseContext();
        if (renderer?._canvas) { renderer._canvas.width = 0; renderer._canvas.height = 0; }
        addon.dispose();
      } catch { /* A failed GPU context must still fall back to DOM. */ }
      addon = null;
    }
    if (counted) { contexts--; counted = false; }
  };
  try {
    addon = new WebglAddon();
    loss = addon.onContextLoss(() => { dispose(); refit(); terminal.refresh(0, terminal.rows - 1); });
    terminal.loadAddon(addon);
    contexts++; counted = true;
    terminal.refresh(0, terminal.rows - 1);
    refit();
  } catch { dispose(); }
  return dispose;
}
